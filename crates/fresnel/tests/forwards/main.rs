//! A port forward from the window on the real core (plan sections 12.3 and 15, M5): the
//! Forward button of a port, the dialog, the Forwards tab and the status bar, the bytes of a
//! local client, a break of the network and a local port in use.
//!
//! The core reads a fake API server through a fault proxy, whose pod sends each byte of each
//! port back. The test draws the frames with [`fresnel::frame::draw`] and uses real time.

#![allow(clippy::expect_used, reason = "a test stops with a panic")]

#[path = "../common/mod.rs"]
mod common;
#[path = "../common/window.rs"]
mod window;

use std::io::{Read as _, Write as _};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::time::{Duration, Instant};

use eframe::egui;
use egui::accesskit::Role;
use egui_kittest::kittest::Queryable as _;
use fresnel_testkit::{FakeApiServer, FaultProxy, Home, ProxyMode};
use tokio::runtime::{Builder, Runtime};

use window::{Window, core, shows, stop_with_no_ended_task, until, window};

const PODS: &str = "/api/v1/pods";
/// The pod of the test: it runs, it is ready, and its container has the port 8080 `http`.
const POD: &str = r#"{"metadata":{"resourceVersion":"7"},"items":[{"metadata":{"name":"web","namespace":"shop",
    "uid":"u1","resourceVersion":"5"},"spec":{"containers":[{"name":"app","ports":[{"name":"http","containerPort":8080}]}]},
    "status":{"phase":"Running","conditions":[{"type":"Ready","status":"True"}]}}]}"#;

struct Bed {
    window: Window,
    proxy: FaultProxy,
    _server: FakeApiServer,
    _home: Home,
    _runtime: Runtime,
}

fn bed() -> Bed {
    let runtime: Runtime = Builder::new_multi_thread().worker_threads(4).enable_all().build().expect("a runtime");
    let (server, proxy) = runtime.block_on(async {
        let server = FakeApiServer::start().await.expect("a server");
        server.set_body(PODS, &POD.replace('\n', ""));
        let proxy = FaultProxy::start(server.addr()).await.expect("a proxy");
        (server, proxy)
    });
    let home = Home::new("forwards", &["kubeconfig.yaml"]).expect("a home folder");
    let (address, authority) = (proxy.addr(), common::authority(&server));
    let text = format!(
        "current-context: ams\nclusters:\n- {{name: ams, cluster: {{server: 'https://{address}', certificate-authority-data: {authority}}}}}\ncontexts:\n- {{name: ams, context: {{cluster: ams}}}}\n"
    );
    home.write("work/kubeconfig.yaml", &text).expect("the kubeconfig");
    let mut window = window(core(&runtime, &home), egui::vec2(1280.0, 800.0));
    until(&mut window, "the pod", |window| shows(window, "1 pod"));
    Bed { window, proxy, _server: server, _home: home, _runtime: runtime }
}

/// Opens the dialog of a forward with the Forward button of port 8080 of the pod `web`.
fn open_dialog(window: &mut Window) {
    window.get_by_role_and_label(Role::Label, "web").click();
    let button = "Forward port 8080 of app";
    until(window, "the panel of web", |window| window.query_by_label(button).is_some());
    window.get_by_label(button).scroll_to_me();
    window.run();
    window.get_by_label(button).click();
    window.run();
    assert!(shows(window, "Pod shop/web in ams"));
}

/// The local addresses in the Forwards tab, as "127.0.0.1:P, [::1]:P". Gives the first one.
fn address(window: &Window) -> Option<SocketAddr> {
    let cells = window.query_all_by_label_contains("127.0.0.1:");
    let texts: Vec<String> = cells.filter_map(|node| node.value()).collect();
    texts.iter().find_map(|text| text.split(", ").next()?.parse().ok())
}

/// Sends bytes through the local port and reads them back from the pod.
fn echo(address: SocketAddr, data: &[u8]) -> std::io::Result<()> {
    let mut client = TcpStream::connect(address)?;
    client.set_read_timeout(Some(Duration::from_secs(20)))?;
    client.write_all(data)?;
    let mut back = vec![0_u8; data.len()];
    client.read_exact(&mut back)?;
    if back == data { Ok(()) } else { Err(std::io::Error::other("other bytes came back")) }
}

/// The way of a user: the Forward button of a port, the dialog, the tab with the address,
/// the bytes of a client, a break of the network that the tab and the status bar show while
/// the local port stays open, and the bytes again after the network returns.
#[test]
fn a_forward_from_the_window_carries_bytes_shows_a_break_and_recovers() {
    let mut bed = bed();
    let window = &mut bed.window;
    open_dialog(window);
    window.get_by_label("Automatic").click();
    window.run();
    window.get_by_label("Start the forward").click();
    until(window, "an active forward", |window| shows(window, "1 forward: 1 active") && address(window).is_some());
    assert!(shows(window, "web:8080"), "the target pod in the tab");
    window.get_by_label("Close").click();
    window.run();
    let address = address(window).expect("a local address");
    echo(address, b"GET / HTTP/1.0\r\n\r\n").expect("the bytes of the pod");
    until(window, "the counts", |window| shows(window, "0 open, 1 in all") && shows(window, "18 sent, 18 received"));

    bed.proxy.set(ProxyMode::Freeze);
    let silent = Instant::now();
    let reconnecting = until(window, "the break", |window| shows(window, "1 forward: 1 reconnecting"));
    assert!(reconnecting <= Duration::from_secs(20), "the break showed after {reconnecting:?}");
    assert!(TcpStream::connect(address).is_ok(), "the local port stays open in the break");
    println!("the window showed the break {:?} after the start of the silence", silent.elapsed());

    bed.proxy.set(ProxyMode::Pass);
    let back = Instant::now();
    until(window, "the forward again", |window| shows(window, "1 forward: 1 active"));
    let started = Instant::now();
    while let Err(error) = echo(address, b"again") {
        assert!(started.elapsed() < Duration::from_secs(30), "the bytes after the break: {error}");
        window.run();
    }
    assert!(back.elapsed() <= Duration::from_secs(10), "the forward worked again after {:?}", back.elapsed());
    stop_with_no_ended_task(window);
}

/// Plan section 15, M5: a local port in use gives the state "failed" with that reason.
#[test]
fn a_local_port_in_use_shows_the_state_failed_with_its_reason() {
    let mut bed = bed();
    let window = &mut bed.window;
    let holder = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("a listener");
    let port = holder.local_addr().expect("its address").port();
    open_dialog(window);
    window.get_by_label("Fixed").click();
    window.run();
    window.get_by_label("Fixed local port").click();
    window.run();
    window.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    window.get_by_label("Fixed local port").type_text(&port.to_string());
    window.run();
    window.get_by_label("Start the forward").click();
    let text = format!("failed: the local port {port} is in use: an other program listens on 127.0.0.1:{port}");
    until(window, "the state failed", |window| shows(window, &text));
    assert!(shows(window, "1 forward: 1 failed"), "the status bar");
    stop_with_no_ended_task(window);
}
