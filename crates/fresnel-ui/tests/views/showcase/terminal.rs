//! The picture of a terminal: a shell in a pod of checkout-api of prod-eu, which the Terminal
//! action of its panel opened. The scene feeds the output of bash into the terminal model of
//! the core, as the session does, so the widget paints what a real session gives.

use std::sync::Arc;
use std::time::SystemTime;

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable as _;
use fresnel_core::app::Command;
use fresnel_core::store::ObjectKey;
use fresnel_core::terminal::{Options, SessionState, Terminal, TerminalRequest, TerminalView};
use fresnel_ui::{TerminalFonts, add_terminal_fonts};
use jiff::Timestamp;
use k8s_openapi::api::core::v1::Pod;

use super::stores::Stores;
use super::usage::usage;
use super::world::{CHECKOUT, PROD_EU};
use super::{save, window};
use crate::fake::{Window, grow_dock, now, shows, until};
use crate::metrics::POLLS;
use crate::workloads::scene::open;

/// The dock grows this much: the terminal has room for the session, and the page above it
/// shows the pods of checkout-api.
const TERMINAL_DOCK: f32 = 180.0;
/// The working folder of the image of checkout-api, and the user of its process.
const FOLDER: &str = "/srv/checkout";
const USER: &str = "app";
/// The memory of a node of the world in KiB, as `node` of `objects` gives its capacity.
const NODE_MEMORY: f64 = 8_148_668.0;

/// The prompt of bash in a Debian image: the title of the terminal, then the user, the host
/// and the folder in color. Bash turns the bracketed paste on before each prompt.
fn prompt(host: &str) -> String {
    let title = format!("\x1b]0;{USER}@{host}: {FOLDER}\x07");
    format!("\x1b[?2004h{title}\x1b[01;32m{USER}@{host}\x1b[00m:\x1b[01;34m{FOLDER}\x1b[00m$ ")
}

/// The lines of `ls -l --color` in the folder of the image: GNU ls in the C locale, with
/// the colors of a folder, a program and a link.
const LISTING: &[&str] = &[
    "total 41212",
    "-rw-r--r-- 1 app app        7 Jan 12 04:51 VERSION",
    "-rwxr-xr-x 1 app app 42183064 Jan 12 04:51 \x1b[01;32mcheckout-api\x1b[0m",
    "drwxr-xr-x 2 app app     4096 Jan 12 04:51 \x1b[01;34mmigrations\x1b[0m",
    "drwxr-xr-x 4 app app     4096 Jan 12 04:51 \x1b[01;34mstatic\x1b[0m",
    "drwxr-xr-x 2 app app     4096 Jan 12 04:51 \x1b[01;34mtemplates\x1b[0m",
    "lrwxrwxrwx 1 app app       13 Jan 12 04:51 \x1b[01;36mtmp\x1b[0m -> \x1b[01;34m/tmp/checkout\x1b[0m",
];

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
        rss as f64 / NODE_MEMORY * 100.0,
        time / 60,
        time % 60
    );
    vec![
        "USER         PID %CPU %MEM    VSZ   RSS TTY      STAT START   TIME COMMAND".to_owned(),
        program,
        "app           27  0.0  0.0   4612  3840 pts/0    Ss   07:59   0:00 bash".to_owned(),
        "app           41  0.0  0.0   8100  4096 pts/0    R+   08:00   0:00 ps aux".to_owned(),
    ]
}

/// The lines of `env | sort | grep _SERVICE_HOST`: the address of each Service of the
/// namespace of the pod, as the kubelet gives it, with the hit of grep in its color.
fn service_hosts(namespace: &str) -> Vec<String> {
    let hit = "\x1b[01;31m\x1b[K_SERVICE_HOST\x1b[m\x1b[K";
    let mut hosts = vec![format!("KUBERNETES{hit}=10.96.0.1")];
    for service in
        Stores::of(&PROD_EU).services.iter().filter(|service| service.metadata.namespace.as_deref() == Some(namespace))
    {
        let address = service.spec.as_ref().and_then(|spec| spec.cluster_ip.as_deref()).expect("an address");
        let name = service.metadata.name.as_deref().expect("a name");
        hosts.push(format!("{}{hit}={address}", name.to_uppercase().replace('-', "_")));
    }
    hosts.sort();
    hosts
}

/// The answer of the readiness endpoint of checkout-api, with the headers of curl.
fn ready() -> Vec<String> {
    let body = r#"{"status":"ready","version":"1.42.3","checks":{"cart":"ok","redis":"ok","payments":"ok"}}"#;
    vec![
        "HTTP/1.1 200 OK".to_owned(),
        "Content-Type: application/json".to_owned(),
        "Date: Fri, 15 Jan 2027 08:00:00 GMT".to_owned(),
        format!("Content-Length: {}", body.len() + 1),
        String::new(),
        body.to_owned(),
    ]
}

/// The bytes of the session: four commands with their output, and the prompt after them. The
/// terminal echoes each command, and bash turns the bracketed paste off before it runs.
fn session(pod: &Pod) -> Vec<u8> {
    let host = pod.metadata.name.as_deref().unwrap_or_default();
    let commands: [(&str, Vec<String>); 4] = [
        ("ls -l --color", LISTING.iter().map(|line| (*line).to_owned()).collect()),
        ("ps aux", processes(pod)),
        ("env | sort | grep _SERVICE_HOST", service_hosts(CHECKOUT.namespace)),
        ("curl -si localhost:8080/ready", ready()),
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

/// The first pod of checkout-api of prod-eu, by name.
fn first_pod(window: &Window) -> Arc<Pod> {
    let of_checkout = |pod: &Pod| pod.metadata.name.as_deref().is_some_and(|name| name.starts_with("checkout-api-"));
    let name = super::pods_of(window, PROD_EU.name, of_checkout).into_iter().next().expect("a pod of checkout-api");
    let clusters = &window.state().view.clusters;
    let data = clusters.iter().find(|row| row.id.as_str() == PROD_EU.name).and_then(|row| row.data.as_ref());
    let pods = &data.expect("the data of prod-eu").pods.items;
    Arc::clone(pods.get(&ObjectKey::new(CHECKOUT.namespace, &name)).expect("the pod"))
}

/// A shell in the first pod of checkout-api of prod-eu: `ls`, `ps`, the addresses of the
/// Services and the readiness endpoint, in an open session. The terminal has the fonts of
/// the system, as the app adds them.
#[test]
fn a_shell_in_a_pod() {
    let mut window = window(egui::Theme::Dark);
    add_terminal_fonts(&window.ctx, TerminalFonts::of_system().0);
    let pod = first_pod(&window);
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
