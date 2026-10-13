//! A disk that is slow: the read of a kubeconfig file or of a certificate file waits.
//!
//! A pipe stands for the file. A read of a pipe waits until the test writes to it.

use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::task::{Context, Wake, Waker};
use std::time::{Duration, Instant};

use fresnel_core::app::{CoreSettings, Health, Startup};
use fresnel_testkit::FakeApiServer;

use super::{App, KUBECONFIG, home, kubeconfig_text, states};
use crate::bed::{authority, certificate_pem};

fn make_pipe(path: &Path) {
    let made = std::process::Command::new("mkfifo").arg(path).status().expect("mkfifo starts");
    assert!(made.success(), "{made}");
}

/// Waits until the core is in the read of the pipe: only then the pipe opens for a write.
fn writer_of(pipe: &Path) -> File {
    let open = || OpenOptions::new().write(true).custom_flags(nix::libc::O_NONBLOCK).open(pipe).ok();
    poll("the core reads the file", open)
}

/// Waits on the thread of the test until `probe` gives a value. The wait uses no timer of
/// the runtime. Thus a test fails, and does not hang, when a read stops the timers.
fn poll<T>(what: &str, mut probe: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(value) = probe() {
            return value;
        }
        assert!(Instant::now() < deadline, "in time: {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Records the wake of a timer.
struct Fired(AtomicBool);

impl Wake for Fired {
    fn wake(self: Arc<Self>) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// Sets a timer of the runtime and waits until it fires. The test starts no task for it:
/// a new task would wake a thread of the runtime, and that thread would run the timers.
fn a_timer_of_the_runtime_fires() {
    let fired = Arc::new(Fired(AtomicBool::new(false)));
    let waker = Waker::from(Arc::clone(&fired));
    let mut timer = std::pin::pin!(tokio::time::sleep(Duration::from_millis(20)));
    if timer.as_mut().poll(&mut Context::from_waker(&waker)).is_pending() {
        poll("a timer of the runtime fires during the read", || fired.0.load(Ordering::Relaxed).then_some(()));
    }
}

/// Ends a read of a pipe that still waits when a test fails. A runtime does not stop
/// while one of its threads is in a read.
struct Release<'a>(&'a Path);

impl Drop for Release<'_> {
    fn drop(&mut self) {
        drop(OpenOptions::new().write(true).custom_flags(nix::libc::O_NONBLOCK).open(self.0));
    }
}

/// The read of the kubeconfig file waits at the worst moment: the start. The window gets
/// its wake and the timers of the runtime run during the read, and a stop during the read
/// is final.
#[tokio::test(flavor = "multi_thread")]
async fn a_slow_read_of_the_kubeconfig_file_stops_no_wake_and_no_timer_and_a_stop_during_it_is_final() {
    let server = FakeApiServer::start().await.expect("a server");
    let home = home("slow-kubeconfig");
    // The login shell takes some time, so the window of the test paints the state before
    // the read with certainty.
    for profile in [".zprofile", ".bash_profile", ".profile"] {
        home.write(profile, &format!("{}sleep 0.2\n", home.read(profile))).expect("the profile");
    }
    let pipe = home.path(KUBECONFIG);
    make_pipe(&pipe);
    let _release = Release(&pipe);
    let app = App::new(&home, CoreSettings::default());
    let wakes = Arc::new(AtomicU32::new(0));
    app.core.start({
        let wakes = Arc::clone(&wakes);
        move || {
            wakes.fetch_add(1, Ordering::Relaxed);
        }
    });

    // The window of the test reads the state at each wake, as the real window does.
    let mut painted = 0;
    poll("the window gets its wake for the state `Kubeconfig` during the read", || {
        let wake = wakes.load(Ordering::Relaxed);
        let is_new = std::mem::replace(&mut painted, wake) != wake;
        (is_new && app.core.view().startup == Startup::Kubeconfig).then_some(())
    });
    let mut writer = writer_of(&pipe);
    a_timer_of_the_runtime_fires();

    app.core.stop();
    app.start();
    let text = kubeconfig_text(&home, "fake", &[("fake", server.addr(), &server, true)]);
    writer.write_all(text.as_bytes()).expect("the kubeconfig");
    drop(writer);
    // Only a check that nothing occurs waits a fixed time.
    app.network_changed.store(true, Ordering::Relaxed);
    std::thread::sleep(Duration::from_millis(300));
    let read_again = OpenOptions::new().write(true).custom_flags(nix::libc::O_NONBLOCK).open(&pipe);
    assert!(read_again.is_err(), "a stopped core does not start again and reads no file");
    assert_eq!((app.core.view().clusters.len(), server.counters().connections), (0, 0));
    assert_eq!(home.read("state/runs"), "", "no plugin runs for a stopped core");
    assert!(app.network_changed.load(Ordering::Relaxed), "no detector looks for a stopped core");
}

/// Weakness 4 of Headlamp: an error of one cluster changes the view of all clusters. Here
/// the certificate file of one cluster is slow, and the other cluster connects with no delay.
#[tokio::test(flavor = "multi_thread")]
async fn a_slow_read_of_the_certificate_file_of_one_cluster_does_not_delay_the_other_cluster() {
    let server = FakeApiServer::start().await.expect("a server");
    let home = home("slow-certificate");
    let pipe = home.path("work/authority.pem");
    make_pipe(&pipe);
    let (address, data) = (server.addr(), authority(&server));
    let text = format!(
        "clusters:
- {{name: fast, cluster: {{server: 'https://{address}', certificate-authority-data: {data}}}}}
- {{name: slow, cluster: {{server: 'https://{address}', certificate-authority: authority.pem}}}}
contexts:
- {{name: fast, context: {{cluster: fast}}}}
- {{name: slow, context: {{cluster: slow}}}}
"
    );
    home.write(KUBECONFIG, &text).expect("the kubeconfig");
    let app = App::new(&home, CoreSettings::default());
    // The two commands wait for the start, so the task of the core runs them in one step.
    app.select("fast", true);
    app.select("slow", true);
    app.start();

    let mut writer = writer_of(&pipe);
    let during = app.until_health("fast", Health::Live).await;
    assert_eq!(states(&during), [("fast", true, Health::Live), ("slow", true, Health::Off)]);
    assert_eq!(app.texts_of("fast"), ["the channels connect", "live: a request got an answer on each channel"]);

    writer.write_all(certificate_pem(&server).as_bytes()).expect("the certificate");
    drop(writer);
    app.until_health("slow", Health::Live).await;
}
