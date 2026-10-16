//! The port forwards (plan section 12.3, M5): the Forwards tab with each state of a forward
//! and its buttons, the dialog of the Forward action and of the Forward button of a port,
//! the status bar and the keys.
//!
//! - `tab`: the tab and the status bar. `dialog`: the dialog and the ways to it.
//! - `snapshots`: the PNG files of the tab and of the dialog.

mod dialog;
mod snapshots;
mod tab;

use std::net::SocketAddr;
use std::sync::Arc;

use egui_kittest::kittest::Queryable as _;
use fresnel_core::app::{ClusterRow, Command, CoreView, ForwardRow};
use fresnel_core::cluster::ClusterId;
use fresnel_core::forward::{
    Bind, Bytes, Connections, ForwardRule, ForwardState, ForwardView, LocalPort, TargetData, TargetPod,
};
use fresnel_core::kinds::Kind;
use fresnel_core::store::ObjectKey;

use crate::data::{held, live, pods};
use crate::fake::{SECOND, Window, now, view, window};

/// A rule to an object of the namespace `shop`.
fn rule(cluster: &str, kind: Kind, name: &str, port: u16, local: LocalPort) -> ForwardRule {
    let key = ObjectKey::new("shop", name);
    ForwardRule { cluster: ClusterId::new(cluster), kind, key, port, local, bind: Bind::Localhost }
}

fn addresses(texts: &[&str]) -> Vec<SocketAddr> {
    texts.iter().map(|text| text.parse().expect("an address")).collect()
}

fn pod(name: &str, port: u16, ready: bool) -> Option<TargetPod> {
    Some(TargetPod { pod: ObjectKey::new("shop", name), uid: Arc::from(name), port, ready })
}

/// A forward of each state: an active Service, a Deployment that reconnects, a Pod whose
/// fixed port is in use, and a StatefulSet with no pod that runs.
fn forwards() -> Vec<ForwardRow> {
    let web = ForwardView {
        addresses: addresses(&["127.0.0.1:8080", "[::1]:8080"]),
        target: pod("web-6888b778b6-75md7", 8080, true),
        data: TargetData::Current,
        connections: Connections { open: 2, total: 14, reset: 0 },
        bytes: Bytes { sent: 12 * 1024 * 1024, received: 300 * 1024 },
        ..ForwardView::new(
            rule("ams-prod", Kind::Service, "web", 80, LocalPort::Fixed(8080)),
            ForwardState::Active,
            now(),
        )
    };
    let reconnecting = ForwardState::Reconnecting {
        attempt: Some(3),
        next_retry: Some(now() + 4 * SECOND),
        reason: "requests: connect: no answer in 10s".to_owned(),
    };
    let api = ForwardView {
        addresses: addresses(&["127.0.0.1:49152", "[::1]:49152"]),
        target: pod("api-7d9c5b8f6-gn8jp", 9090, true),
        data: TargetData::Old { kind: Kind::Pod, since: now() - 48 * SECOND },
        last_error: Some((now() - 12 * SECOND, "a tunnel broke: silent: no frame for 15.0 s".to_owned())),
        connections: Connections { open: 0, total: 3, reset: 1 },
        ..ForwardView::new(rule("fra-prod", Kind::Deployment, "api", 9090, LocalPort::Automatic), reconnecting, now())
    };
    let in_use = "the local port 5432 is in use: an other program listens on 127.0.0.1:5432".to_owned();
    let db = ForwardView::new(
        rule("ams-prod", Kind::Pod, "db-0", 5432, LocalPort::Fixed(5432)),
        ForwardState::Failed(in_use),
        now(),
    );
    let no_pod = ForwardState::Degraded("the StatefulSet shop/cache has no pod that runs".to_owned());
    let cache = ForwardView {
        addresses: addresses(&["127.0.0.1:6379", "[::1]:6379"]),
        data: TargetData::Current,
        ..ForwardView::new(rule("ams-prod", Kind::StatefulSet, "cache", 6379, LocalPort::Fixed(6379)), no_pod, now())
    };
    let row = |id, saved, view| ForwardRow { id, saved, view: held(view) };
    vec![row(1, true, web), row(2, false, api), row(3, false, db), row(4, false, cache)]
}

/// The view of two live clusters with these forwards.
fn with_forwards(rows: Vec<ForwardRow>) -> CoreView {
    let clusters: Vec<ClusterRow> = vec![live("ams-prod", &pods()), live("fra-prod", &pods())];
    CoreView { forwards: rows.into(), ..view(clusters) }
}

/// The window with these forwards and the Forwards tab open.
fn tab_window(rows: Vec<ForwardRow>) -> Window {
    let mut window = window(with_forwards(rows));
    window.get_by_label("Forwards").click();
    window.run();
    window
}

/// The commands that the views sent since the last call.
fn sent(window: &mut Window) -> Vec<Command> {
    std::mem::take(&mut window.state_mut().sent)
}
