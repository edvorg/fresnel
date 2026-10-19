//! The picture of the Forwards tab: four forwards to objects of the world, each in the state
//! that the core gives it from the stores and the connections of its cluster.
//!
//! - The Service checkout-api of prod-eu is active: its first ready pod takes the tunnels.
//! - The Service prometheus of prod-us reconnects with prod-us, and keeps its pod of before.
//! - A pod of cart of prod-eu is degraded: the drain evicted it.
//! - The Deployment checkout-api of staging is active with the pod that crashes, which runs
//!   and is not ready: a user forwards to such a pod to find its fault.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::SystemTime;

use egui_kittest::kittest::Queryable as _;
use fresnel_core::app::ForwardRow;
use fresnel_core::cluster::{ClusterId, LinkState};
use fresnel_core::forward::{
    Bind, Bytes, Connections, ForwardRule, ForwardState, ForwardView, LocalPort, TargetData, TargetPod,
};
use fresnel_core::kinds::Kind;
use fresnel_core::net::ChannelState;
use fresnel_core::store::ObjectKey;
use k8s_openapi::api::core::v1::Pod;

use super::clusters::{LOST, PROD_US_REASON};
use super::world::{CART, CHECKOUT, PROD_EU, PROD_US, STAGING, evicted};
use super::{save, uid_of, window};
use crate::data::held;
use crate::fake::{SECOND, Window, assert_dragged, grow_dock, move_edge, now, open_tab, shows, until};
use crate::frame_time::wheel;
use crate::kinds::in_panel;
use crate::workloads::scene::open;

/// The dock grows this much, so the chosen forward shows each of its lines.
const FORWARDS_DOCK: f32 = 210.0;
/// The chosen forward gets this much narrower, so the pod of each forward fits in the list.
const CHOSEN_NARROWING: f32 = 70.0;
/// The chosen forward has a margin of this many points at its left: its edge is that far left
/// of its text.
const CHOSEN_MARGIN: f32 = 8.0;
const MI: u64 = 1024 * 1024;

/// A rule to the port of an object, on the local addresses of the loopback.
fn rule(cluster: &str, (kind, namespace, name): (Kind, &str, &str), port: u16, local: LocalPort) -> ForwardRule {
    let key = ObjectKey::new(namespace, name);
    ForwardRule { cluster: ClusterId::new(cluster), kind, key, port, local, bind: Bind::Localhost }
}

/// The two addresses of the loopback with this port.
fn loopback(port: u16) -> Vec<SocketAddr> {
    vec![SocketAddr::from(([127, 0, 0, 1], port)), SocketAddr::from((std::net::Ipv6Addr::LOCALHOST, port))]
}

/// A pod of the world as the target of the tunnels.
fn target(cluster: &str, (namespace, name): (&str, &str), port: u16, ready: bool) -> Option<TargetPod> {
    let uid = uid_of(cluster, Kind::Pod, namespace, name);
    Some(TargetPod { pod: ObjectKey::new(namespace, name), uid: uid.into(), port, ready })
}

/// The first pod of an app in a cluster, by name: a pod of a Service or a workload with no
/// time of its readiness, as the core picks it.
fn first_pod(window: &Window, cluster: &str, app: &str) -> String {
    let of_app =
        |pod: &Pod| pod.metadata.labels.as_ref().and_then(|labels| labels.get("app")).is_some_and(|name| name == app);
    super::pods_of(window, cluster, of_app).into_iter().next().expect("a pod of the app")
}

/// The attempt of prod-us and the time of the next one, as a forward takes them from the
/// first channel of its requests: its failures and the end of its pause.
fn next_attempt(window: &Window) -> (Option<u32>, Option<SystemTime>) {
    let row = window.state().view.clusters.iter().find(|row| row.id.as_str() == PROD_US.name);
    let Some(LinkState::On(status)) = row.map(|row| &row.link) else {
        panic!("prod-us has a connection");
    };
    let channel = status.requests.channels.first().expect("a channel of the requests");
    let ChannelState::Backoff { failures, pause } = channel.state else {
        panic!("the channel of prod-us waits for its next attempt");
    };
    (Some(failures + 1), channel.since.checked_add(pause))
}

/// Drags the left edge of the chosen forward `by` points to the right. Its title is the target
/// of the forward, which is in the list too: the title is the label at the right.
fn narrow_chosen(window: &mut Window, target: &str, by: f32) {
    let left = |window: &Window| {
        window.query_all_by_label(target).map(|label| label.rect().left()).reduce(f32::max).expect("the title")
    };
    let before = left(window);
    let row = window.query_all_by_label(target).map(|label| label.rect().center().y).reduce(f32::max);
    let edge = egui::pos2(before - CHOSEN_MARGIN, row.expect("the title"));
    move_edge(window, edge, edge + egui::vec2(by, 0.0));
    assert_dragged(window, "the left of the chosen forward", left(window) - before, by);
}

/// Scrolls the panel of the Service to its ports, with the button that starts a forward, and
/// its endpoints.
fn to_the_ports(window: &mut Window) {
    let top = |window: &Window, heading: &str| window.get_by_label(heading).rect().top();
    let first = top(window, "METADATA");
    window.hover_at(window.get_by_label("METADATA").rect().center());
    window.event(wheel(first - top(window, "PORTS")));
    window.run();
}

/// The four forwards of the scene.
fn forwards(window: &Window) -> Vec<ForwardRow> {
    let checkout = ForwardView {
        addresses: loopback(8080),
        target: target(PROD_EU.name, (CHECKOUT.namespace, &first_pod(window, PROD_EU.name, CHECKOUT.name)), 8080, true),
        data: TargetData::Current,
        connections: Connections { open: 2, total: 57, reset: 0 },
        bytes: Bytes { sent: 1_400_000, received: 23 * MI },
        ..ForwardView::new(
            rule(PROD_EU.name, (Kind::Service, CHECKOUT.namespace, CHECKOUT.name), 8080, LocalPort::Fixed(8080)),
            ForwardState::Active,
            now() - 1260 * SECOND,
        )
    };
    let lost = now() - LOST * SECOND;
    let (attempt, next_retry) = next_attempt(window);
    let reconnecting = ForwardState::Reconnecting { attempt, next_retry, reason: PROD_US_REASON.to_owned() };
    let broke = "a tunnel broke: silent: no frame for 15.0 s, also no answer to a ping";
    let prometheus = ForwardView {
        addresses: loopback(9090),
        target: target(PROD_US.name, ("monitoring", &first_pod(window, PROD_US.name, "prometheus")), 9090, true),
        data: TargetData::Old { kind: Kind::Pod, since: lost },
        last_error: Some((lost, broke.to_owned())),
        connections: Connections { open: 0, total: 214, reset: 1 },
        bytes: Bytes { sent: 3 * MI, received: 41 * MI },
        ..ForwardView::new(
            rule(PROD_US.name, (Kind::Service, "monitoring", "prometheus"), 9090, LocalPort::Fixed(9090)),
            reconnecting,
            lost,
        )
    };
    // The pod of cart on the node of the drain is gone.
    let gone = evicted(&CART.replica_set().expect("a Deployment"));
    let cart = ForwardView {
        addresses: loopback(8081),
        data: TargetData::Current,
        connections: Connections { open: 0, total: 9, reset: 0 },
        bytes: Bytes { sent: 18 * 1024, received: 236 * 1024 },
        ..ForwardView::new(
            rule(PROD_EU.name, (Kind::Pod, "shop", &gone), 8080, LocalPort::Fixed(8081)),
            ForwardState::Degraded { reason: format!("the Pod shop/{gone} does not exist"), last_failure: None },
            now() - 75 * SECOND,
        )
    };
    let candidate = first_pod(window, STAGING.name, CHECKOUT.name);
    let staging = ForwardView {
        addresses: loopback(53817),
        target: target(STAGING.name, (CHECKOUT.namespace, &candidate), 8080, false),
        data: TargetData::Current,
        ..ForwardView::new(
            rule(STAGING.name, (Kind::Deployment, CHECKOUT.namespace, CHECKOUT.name), 8080, LocalPort::Automatic),
            ForwardState::Active,
            now() - 20 * SECOND,
        )
    };
    let row = |id, view| ForwardRow { id, saved: false, view: held(view) };
    vec![row(1, checkout), row(2, prometheus), row(3, cart), row(4, staging)]
}

/// The Services of the three clusters with the panel of checkout-api of prod-eu, and the
/// Forwards tab with the forward to the pod that the drain evicted chosen.
#[test]
fn port_forwards_in_each_state() {
    let mut window = window(egui::Theme::Dark);
    open(&mut window, "Services", CHECKOUT.name);
    assert!(in_panel(&window, PROD_EU.name), "the first row of the filter is the Service of prod-eu");
    super::panels::set_filter(&mut window, "", "18 Services");
    let rows = forwards(&window);
    let cart = rows.get(2).expect("the forward of cart").view.clone();
    window.state_mut().view.forwards = Arc::from(rows);
    open_tab(&mut window, "Forwards");
    grow_dock(&mut window, FORWARDS_DOCK);
    to_the_ports(&mut window);
    window.get_by_label(&format!("Forward of {}", cart.rule)).click();
    let state = format!("degraded: the Pod shop/{} does not exist", cart.rule.key.name);
    until(&mut window, "the chosen forward", |window| shows(window, &state));
    narrow_chosen(&mut window, &cart.rule.target(), CHOSEN_NARROWING);
    save(&mut window, "port-forwards");
}
