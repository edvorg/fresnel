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
use std::time::Duration;

use egui_kittest::kittest::Queryable as _;
use fresnel_core::app::ForwardRow;
use fresnel_core::cluster::{ClusterId, LinkState};
use fresnel_core::forward::{
    Bind, Bytes, Connections, ForwardRule, ForwardState, ForwardView, LocalPort, TargetData, TargetPod,
};
use fresnel_core::kinds::Kind;
use fresnel_core::net::ChannelState;
use fresnel_core::store::ObjectKey;

use super::clusters::LOST;
use super::panels::{scroll_panel_to, set_filter};
use super::world::{CART, CART_REPLACED, CHECKOUT, PROD_EU, PROD_US, STAGING, evicted};
use super::{first_pod, save, uid_of, window};
use crate::data::held;
use crate::fake::{SECOND, Window, drag_chosen, grow_dock, now, open_tab, shows, until};
use crate::kinds::in_panel;
use crate::workloads::scene::open;

/// The dock grows this much, so the chosen forward shows each of its lines.
const FORWARDS_DOCK: f32 = 210.0;
/// The chosen forward gets this much narrower, so the pod of each forward fits in the list.
const CHOSEN_NARROWING: f32 = 70.0;
const MI: u64 = 1024 * 1024;
/// The seconds since a user opened the forward of checkout-api: its tunnels carried 57
/// connections since then.
const CHECKOUT_OPENED: u32 = 21 * 60;
/// The seconds that the evicted pod of cart took to end: then the store lost it, and its
/// forward became degraded.
const CART_EXIT: u64 = 4;
/// The seconds since a user opened the forward to staging, to find why its pod crashes.
const STAGING_OPENED: u32 = 20;

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

/// The name of the first pod of an app in a cluster.
fn first_pod_name(window: &Window, cluster: &str, app: &str) -> String {
    first_pod(window, cluster, app).metadata.name.clone().expect("a name")
}

/// The state of a forward of prod-us, as the core gives it from the row of the cluster: the
/// attempt and the time of the next one from the first channel of its requests (its failures
/// and the end of its pause), and the reason why the cluster is not live.
fn reconnecting(window: &Window) -> ForwardState {
    let row = window.state().view.clusters.iter().find(|row| row.id.as_str() == PROD_US.name);
    let row = row.expect("the row of prod-us");
    let LinkState::On(status) = &row.link else {
        panic!("prod-us has a connection");
    };
    let channel = status.requests.channels.first().expect("a channel of the requests");
    let ChannelState::Backoff { failures, pause } = channel.state else {
        panic!("the channel of prod-us waits for its next attempt");
    };
    let reason = row.reason().expect("prod-us is not live");
    ForwardState::Reconnecting { attempt: Some(failures + 1), next_retry: channel.since.checked_add(pause), reason }
}

/// Drags the left edge of the chosen forward `by` points to the right. Its title is the target
/// of the forward, which is in the list too: the title is the label at the right, and the
/// press is at the height of the row of the list, the lower label.
fn narrow_chosen(window: &mut Window, target: &str, by: f32) {
    let labels = |window: &Window| window.query_all_by_label(target).map(|label| label.rect()).collect::<Vec<_>>();
    let title = |window: &Window| labels(window).iter().map(|label| label.left()).reduce(f32::max).expect("the title");
    let row = labels(window).iter().map(|label| label.center().y).reduce(f32::max).expect("the row");
    drag_chosen(window, title, row, -by);
}

/// The four forwards of the scene.
fn forwards(window: &Window) -> Vec<ForwardRow> {
    let picked = first_pod_name(window, PROD_EU.name, CHECKOUT.name);
    let checkout = ForwardView {
        addresses: loopback(8080),
        target: target(PROD_EU.name, (CHECKOUT.namespace, &picked), 8080, true),
        data: TargetData::Current,
        connections: Connections { open: 2, total: 57, reset: 0 },
        bytes: Bytes { sent: 1_400_000, received: 23 * MI },
        ..ForwardView::new(
            rule(PROD_EU.name, (Kind::Service, CHECKOUT.namespace, CHECKOUT.name), 8080, LocalPort::Fixed(8080)),
            ForwardState::Active,
            now() - CHECKOUT_OPENED * SECOND,
        )
    };
    let lost = now() - LOST * SECOND;
    let broke = "a tunnel broke: silent: no frame for 15.0 s, also no answer to a ping";
    let prometheus = ForwardView {
        addresses: loopback(9090),
        target: target(PROD_US.name, ("monitoring", &first_pod_name(window, PROD_US.name, "prometheus")), 9090, true),
        data: TargetData::Old { kind: Kind::Pod, since: lost },
        last_error: Some((lost, broke.to_owned())),
        connections: Connections { open: 0, total: 214, reset: 1 },
        bytes: Bytes { sent: 3 * MI, received: 41 * MI },
        ..ForwardView::new(
            rule(PROD_US.name, (Kind::Service, "monitoring", "prometheus"), 9090, LocalPort::Fixed(9090)),
            reconnecting(window),
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
            now() - Duration::from_secs(CART_REPLACED - CART_EXIT),
        )
    };
    let candidate = first_pod_name(window, STAGING.name, CHECKOUT.name);
    let staging = ForwardView {
        addresses: loopback(53817),
        target: target(STAGING.name, (CHECKOUT.namespace, &candidate), 8080, false),
        data: TargetData::Current,
        ..ForwardView::new(
            rule(STAGING.name, (Kind::Deployment, CHECKOUT.namespace, CHECKOUT.name), 8080, LocalPort::Automatic),
            ForwardState::Active,
            now() - STAGING_OPENED * SECOND,
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
    set_filter(&mut window, "", "21 Services");
    let rows = forwards(&window);
    let cart = rows.get(2).expect("the forward of cart").view.clone();
    window.state_mut().view.forwards = Arc::from(rows);
    open_tab(&mut window, "Forwards");
    grow_dock(&mut window, FORWARDS_DOCK);
    // The ports with the button that starts a forward, and the endpoints.
    scroll_panel_to(&mut window, "PORTS");
    window.get_by_label(&format!("Forward of {}", cart.rule)).click();
    let state = format!("degraded: the Pod shop/{} does not exist", cart.rule.key.name);
    until(&mut window, "the chosen forward", |window| shows(window, &state));
    narrow_chosen(&mut window, &cart.rule.target(), CHOSEN_NARROWING);
    save(&mut window, "port-forwards");
}
