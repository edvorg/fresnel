//! The random faults of the soak (plan section 13, line 2): silent connections, resets,
//! repeated resets, held connects, breaks of the network and a restart of the API server,
//! with a pause of the pace between two faults, and a recreate of the cluster at the time of
//! the pace. Each fault is a line of the file of the faults, which the soak adds to its record
//! of the minute. The keeper looks at the API server in each pause.

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use fresnel_testkit::{FaultProxy, K3s, ProxyMode};
use tokio::time::{Instant, sleep};

use super::Never;
use super::keeper::Keeper;
use super::pace::Pace;
use super::scene;

/// How long the modes of the proxy that are not one moment last.
const RESETS_TIME: Duration = Duration::from_secs(60);
const HOLD_TIME: Duration = Duration::from_secs(120);
/// The shortest and the longest break of the network, in seconds.
const BREAK: (u64, u64) = (30, 180);

/// One kind of fault.
#[derive(Clone, Copy)]
enum Fault {
    Silence,
    Reset,
    Resets,
    Held,
    Break,
    Restart,
}

/// Each kind once in each round, in a random sequence: an hour has each kind, and two rounds
/// have it two times. A draw of each fault alone could leave a kind out of a short soak.
const ROUND: [Fault; 6] = [Fault::Silence, Fault::Reset, Fault::Resets, Fault::Held, Fault::Break, Fault::Restart];

pub(crate) struct Faults<'a> {
    k3s: &'a K3s,
    proxy: &'a FaultProxy,
    file: PathBuf,
    rng: fastrand::Rng,
    pace: &'a Pace,
    /// The time of the recreate of the cluster, until it came.
    recreate_at: Option<Instant>,
    keeper: Keeper<'a>,
    /// The faults of the round that did not come yet.
    round: Vec<Fault>,
    made: BTreeMap<&'static str, u64>,
}

impl<'a> Faults<'a> {
    pub(crate) fn new(k3s: &'a K3s, proxy: &'a FaultProxy, file: &Path, rng: fastrand::Rng, pace: &'a Pace) -> Self {
        let (recreate_at, keeper) =
            (pace.recreate_after.map(|after| Instant::now() + after), Keeper::new(k3s, pace.line_pause));
        let (file, round, made) = (file.to_owned(), Vec::new(), BTreeMap::new());
        Self { k3s, proxy, file, rng, pace, recreate_at, keeper, round, made }
    }

    /// Makes faults until the environment cannot be restored: the error tells why.
    pub(crate) async fn run(&mut self) -> Result<Never, String> {
        loop {
            let pause = self.rng.u64(self.pace.fault_pause.0..=self.pace.fault_pause.1);
            self.keeper.wait(&self.file, Duration::from_secs(pause)).await?;
            if self.recreate_at.is_some_and(|at| Instant::now() >= at) {
                self.recreate_at = None;
                self.recreate().await?;
                continue;
            }
            if self.round.is_empty() {
                self.round = ROUND.to_vec();
                self.rng.shuffle(&mut self.round);
            }
            match self.round.pop().unwrap_or(Fault::Reset) {
                Fault::Silence => {
                    self.note("silence", "each open connection is silent");
                    self.proxy.silence_open();
                }
                Fault::Reset => {
                    self.note("reset", "a reset of each connection");
                    self.proxy.reset_all().await;
                }
                Fault::Resets => {
                    self.note("resets", "a reset of each connection each 5 s for 60 s");
                    self.for_a_time(ProxyMode::ResetEach(Duration::from_secs(5)), RESETS_TIME).await;
                }
                Fault::Held => {
                    self.note("held connects", "30 percent of the new connections held 1 to 10 s, for 120 s");
                    let hold = ProxyMode::HoldNew {
                        probability: 0.3,
                        min: Duration::from_secs(1),
                        max: Duration::from_secs(10),
                    };
                    // Each connection is new in that time.
                    self.proxy.reset_all().await;
                    self.for_a_time(hold, HOLD_TIME).await;
                }
                Fault::Break => {
                    let seconds = self.rng.u64(BREAK.0..=BREAK.1);
                    self.note("break", &format!("a break of the network of {seconds} s"));
                    self.for_a_time(ProxyMode::Freeze, Duration::from_secs(seconds)).await;
                }
                Fault::Restart => {
                    self.note("restart", "a restart of the API server");
                    let start = Instant::now();
                    let restarted = self.k3s.restart().await;
                    let result =
                        restarted.as_ref().map_or_else(|error| format!("failed: {error}"), |()| "ready".to_owned());
                    println!("fresnel-soak: the API server is {result} after {:.0} s", start.elapsed().as_secs_f64());
                    self.keeper.after_fault(&self.file, restarted.is_ok()).await?;
                }
            }
        }
    }

    /// A new cluster in the same container: each object of the soak comes again with a new
    /// UID, as after a restore of a cluster from a backup.
    async fn recreate(&mut self) -> Result<(), String> {
        self.note("recreate", "a recreate of the cluster: each object gets a new UID");
        let start = Instant::now();
        let recreated = self.k3s.recreate().await;
        if let Err(error) = &recreated {
            println!("fresnel-soak: the recreate failed: {error}");
        } else {
            scene::create(self.k3s, self.pace.line_pause).await?;
        }
        self.keeper.after_fault(&self.file, recreated.is_ok()).await?;
        println!("fresnel-soak: the cluster is new after {:.0} s", start.elapsed().as_secs_f64());
        Ok(())
    }

    async fn for_a_time(&self, mode: ProxyMode, time: Duration) {
        self.proxy.set(mode);
        sleep(time).await;
        self.proxy.set(ProxyMode::Pass);
    }

    /// Counts a fault and writes its line.
    fn note(&mut self, kind: &'static str, text: &str) {
        *self.made.entry(kind).or_default() += 1;
        println!("fresnel-soak: fault: {text}");
        note_line(&self.file, text);
    }

    pub(crate) fn summary(&self) -> String {
        let faults: Vec<String> = self.made.iter().map(|(kind, count)| format!("{count} {kind}")).collect();
        let faults = if faults.is_empty() { "none".to_owned() } else { faults.join(", ") };
        format!("faults: {faults}; outages of the environment: {}", self.keeper.outages)
    }
}

/// Adds one line to the file of the faults.
pub(crate) fn note_line(file: &Path, text: &str) {
    let written = OpenOptions::new().append(true).open(file).and_then(|mut opened| writeln!(opened, "{text}"));
    if let Err(error) = written {
        eprintln!("fresnel-soak: cannot write {}: {error}", file.display());
    }
}
