//! The picture of a terminal: a shell in a pod of checkout-api of prod-eu, which the Terminal
//! action of its panel opened. The scene feeds the output of bash into the terminal model of
//! the core, as the session does, so the widget paints what a real session gives.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable as _;
use fresnel_core::app::Command;
use fresnel_core::terminal::{Options, SessionState, Terminal, TerminalRequest, TerminalView};
use fresnel_ui::{TerminalFonts, add_terminal_fonts};
use jiff::Timestamp;
use k8s_openapi::api::core::v1::{Pod, Service};

use super::objects::{MINUTE, NODE_MEMORY_KI};
use super::stores::Stores;
use super::usage::usage;
use super::world::{CHECKOUT, PROD_EU, RELEASE_AGE};
use super::{first_pod, save, window};
use crate::fake::{Window, grow_dock, now, shows, until};
use crate::metrics::POLLS;
use crate::workloads::scene::open;

/// The dock grows this much: the terminal has room for the session, and the page above it
/// shows the pods of checkout-api.
const TERMINAL_DOCK: f32 = 180.0;
/// The working folder of the image of checkout-api, and the user of its process.
const FOLDER: &str = "/srv/checkout";
const USER: &str = "app";
/// The seconds since the start of the shell: ps runs at the time of the view tests.
const SHELL_AGE: u64 = 40;

/// The prompt of bash in a Debian image: the title of the terminal, then the user, the host
/// and the folder in color. Bash turns the bracketed paste on before each prompt.
fn prompt(host: &str) -> String {
    let title = format!("\x1b]0;{USER}@{host}: {FOLDER}\x07");
    format!("\x1b[?2004h{title}\x1b[01;32m{USER}@{host}\x1b[00m:\x1b[01;34m{FOLDER}\x1b[00m$ ")
}

/// The time `seconds` before the time of the view tests in a format of strftime, in UTC as
/// the clock of the pod.
fn clock(seconds: u64, format: &str) -> String {
    let time = Timestamp::try_from(now() - Duration::from_secs(seconds)).expect("a time");
    time.strftime(format).to_string()
}

/// The release of checkout-api: the tag of its image.
fn release() -> &'static str {
    CHECKOUT.image.rsplit(':').next().expect("a tag")
}

/// A file of the folder of the image: its mode, its count of links, its size in bytes, and
/// its name in the colors of GNU ls for a program, a folder and a link.
type File = (&'static str, u32, u64, &'static str);

/// The files of the folder after VERSION, which holds the release.
const LISTING: [File; 5] = [
    ("-rwxr-xr-x", 1, 42_183_064, "\x1b[01;32mcheckout-api\x1b[0m"),
    ("drwxr-xr-x", 2, 4096, "\x1b[01;34mmigrations\x1b[0m"),
    ("drwxr-xr-x", 4, 4096, "\x1b[01;34mstatic\x1b[0m"),
    ("drwxr-xr-x", 2, 4096, "\x1b[01;34mtemplates\x1b[0m"),
    ("lrwxrwxrwx", 1, 13, "\x1b[01;36mtmp\x1b[0m -> \x1b[01;34m/tmp/checkout\x1b[0m"),
];
/// The blocks of the file system of the image. The total of `ls -l` adds the KiB of the
/// blocks of each file, and a short link keeps its target in its inode, with no block.
const BLOCK: u64 = 4096;
/// The age of each file: the image was built 28 minutes before staging ran the release first.
const BUILT: u64 = RELEASE_AGE + 28 * MINUTE;

/// The output of `ls -l --color` in the C locale: the total, VERSION with the release and a
/// new line, and `LISTING`.
fn listing() -> Vec<String> {
    let version: File = ("-rw-r--r--", 1, release().len() as u64 + 1, "VERSION");
    let files: Vec<File> = std::iter::once(version).chain(LISTING).collect();
    let kib = |(mode, _, size, _): &File| if mode.starts_with('l') { 0 } else { size.div_ceil(BLOCK) * BLOCK / 1024 };
    let total = format!("total {}", files.iter().map(kib).sum::<u64>());
    let time = clock(BUILT, "%b %e %H:%M");
    let line = |(mode, links, size, name): &File| format!("{mode} {links} {USER} {USER} {size:>8} {time} {name}");
    std::iter::once(total).chain(files.iter().map(line)).collect()
}

/// `ps aux` in the pod: the program of checkout-api with the CPU and the memory of the pod in
/// the metrics, from the start of its container on, the shell and ps. ps prints a value
/// wider than its column as it is, and the columns after it move.
fn processes(pod: &Pod) -> Vec<String> {
    let name = pod.metadata.name.as_deref().unwrap_or_default();
    let used = usage(&CHECKOUT, name, POLLS - 1);
    let cpu = used.cpu as f64 / 1e9;
    let rss = used.memory / 1024;
    let started = pod.metadata.creation_timestamp.as_ref().map_or(now(), |time| SystemTime::from(time.0));
    let run = now().duration_since(started).unwrap_or_default().as_secs_f64();
    let time = (cpu * run) as u64;
    let start = Timestamp::try_from(started).expect("a time").strftime("%b%d");
    let program = format!(
        "app            1 {:.1} {:4.1} 1874516 {rss} ?      Ssl  {start} {}:{:02} {FOLDER}/checkout-api",
        cpu * 100.0,
        rss as f64 / NODE_MEMORY_KI as f64 * 100.0,
        time / 60,
        time % 60
    );
    let (shell, ps) = (clock(SHELL_AGE, "%H:%M"), clock(0, "%H:%M"));
    vec![
        "USER         PID %CPU %MEM    VSZ   RSS TTY      STAT START   TIME COMMAND".to_owned(),
        program,
        format!("app           27  0.0  0.0   4612  3840 pts/0    Ss   {shell}   0:00 bash"),
        format!("app           41  0.0  0.0   8100  4096 pts/0    R+   {ps}   0:00 ps aux"),
    ]
}

/// True for a Service whose address the kubelet gives to a pod of `namespace`: a Service of
/// the namespace, and the Service kubernetes.
fn given_to(namespace: &str) -> impl Fn(&&Arc<Service>) -> bool + '_ {
    move |service| {
        let place = (service.metadata.namespace.as_deref(), service.metadata.name.as_deref());
        place.0 == Some(namespace) || place == (Some("default"), Some("kubernetes"))
    }
}

/// The lines of `env | sort | grep _SERVICE_HOST`: the address of each Service that the
/// kubelet gives to a pod of the namespace, with the hit of grep in its color.
fn service_hosts(namespace: &str) -> Vec<String> {
    let hit = "\x1b[01;31m\x1b[K_SERVICE_HOST\x1b[m\x1b[K";
    let services = Stores::of(&PROD_EU).services;
    let host = |service: &Arc<Service>| {
        let address = service.spec.as_ref().and_then(|spec| spec.cluster_ip.as_deref()).expect("an address");
        let name = service.metadata.name.as_deref().expect("a name");
        format!("{}{hit}={address}", name.to_uppercase().replace('-', "_"))
    };
    let mut hosts: Vec<String> = services.iter().filter(given_to(namespace)).map(host).collect();
    hosts.sort();
    hosts
}

/// The answer of the readiness endpoint of checkout-api, with the headers of curl.
fn ready() -> Vec<String> {
    let checks = r#"{"cart":"ok","redis":"ok","payments":"ok"}"#;
    let body = format!(r#"{{"status":"ready","version":"{}","checks":{checks}}}"#, release());
    vec![
        "HTTP/1.1 200 OK".to_owned(),
        "Content-Type: application/json".to_owned(),
        format!("Date: {}", clock(0, "%a, %d %b %Y %H:%M:%S GMT")),
        format!("Content-Length: {}", body.len() + 1),
        String::new(),
        body,
    ]
}

/// The bytes of the session: four commands with their output, and the prompt after them. The
/// terminal echoes each command, and bash turns the bracketed paste off before it runs.
fn session(pod: &Pod) -> Vec<u8> {
    let host = pod.metadata.name.as_deref().unwrap_or_default();
    let (port, path) = CHECKOUT.readiness.expect("the readiness probe of checkout-api");
    let commands = [
        ("ls -l --color".to_owned(), listing()),
        ("ps aux".to_owned(), processes(pod)),
        ("env | sort | grep _SERVICE_HOST".to_owned(), service_hosts(CHECKOUT.namespace)),
        (format!("curl -si localhost:{port}{path}"), ready()),
    ];
    let mut text = String::new();
    for (command, lines) in commands {
        text.push_str(&prompt(host));
        text.push_str(&format!("{command}\x1b[?2004l\r\r\n"));
        for line in lines {
            text.push_str(&line);
            text.push_str("\r\n");
        }
    }
    text.push_str(&prompt(host));
    text.into_bytes()
}

/// The request of the last terminal that the views asked the core for.
fn opened(window: &Window) -> TerminalRequest {
    let mut sent = window.state().sent.iter().rev();
    let request = sent.find_map(|command| match command {
        Command::OpenTerminal(request) => Some(request.clone()),
        _ => None,
    });
    request.expect("the request of a terminal")
}

/// A shell in the first pod of checkout-api of prod-eu: `ls`, `ps`, the addresses of the
/// Services and the readiness endpoint, in an open session. The terminal has the fonts of
/// the system, as the app adds them.
#[test]
fn a_shell_in_a_pod() {
    let mut window = window(egui::Theme::Dark);
    add_terminal_fonts(&window.ctx, TerminalFonts::of_system().0);
    let pod = first_pod(&window, PROD_EU.name, CHECKOUT.name);
    let name = pod.metadata.name.clone().unwrap_or_default();
    open(&mut window, "Pods", &name);
    super::panels::set_filter(&mut window, CHECKOUT.name, "7 of 46 pods");
    window.get_by_role_and_label(Role::Button, "More").click();
    window.run();
    window.get_by_role_and_label(Role::Button, "Terminal").click();
    window.run();
    let shell = TerminalView { id: 1, request: opened(&window), terminal: Terminal::new(Options::default()) };
    shell.terminal.set_state(SessionState::Open);
    window.state_mut().view.terminals = vec![shell.clone()].into();
    until(&mut window, "the tab of the terminal", |window| window.query_by_label("Terminal").is_some());
    grow_dock(&mut window, TERMINAL_DOCK);
    shell.terminal.feed(&session(&pod));
    window.get_by_label("Terminal").click();
    let title = format!("“{USER}@{name}: {FOLDER}”");
    until(&mut window, "the title of the shell", |window| shows(window, &title));
    save(&mut window, "terminal");
}
