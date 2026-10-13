use std::time::Duration;

use fresnel_core::CoreError;
use fresnel_core::kinds::Kind;
use fresnel_core::net::{ChannelStatus, ConnectStatus, ErrorClass, ThrottleStatus, UpgradeStatus};
use fresnel_core::store::{StoreState, Wait};

use super::*;

const EPOCH: SystemTime = SystemTime::UNIX_EPOCH;
const SECOND: Duration = Duration::from_secs(1);

fn group(states: &[(ChannelState, usize)]) -> GroupStatus {
    let channel = |(state, streams_open): &(ChannelState, usize)| ChannelStatus {
        state: state.clone(),
        since: EPOCH,
        streams_open: *streams_open,
        stream_budget: 70,
        server_stream_limit: Some(100),
        ping_time: None,
        last_frame_age: None,
        connect_attempts: 1,
        connections: 1,
        last_error: None,
    };
    GroupStatus { channels: states.iter().map(channel).collect(), waiting: 0 }
}

fn connections(watches: &[(ChannelState, usize)], requests: ChannelState) -> ClusterStatus {
    let connect = ConnectStatus {
        addresses: Vec::new(),
        resolve_error: None,
        connect_time: None,
        hedge_delay: Duration::from_millis(200),
        connects: 3,
        attempts: 3,
    };
    ClusterStatus {
        name: "ams".to_owned(),
        connect,
        watches: group(watches),
        requests: group(&[(requests, 0)]),
        logs: group(&[(ChannelState::Live, 0)]),
        credential: None,
        upgrade: UpgradeStatus::default(),
        throttle: ThrottleStatus::default(),
    }
}

fn store(kind: Kind, state: StoreState, items: usize, last_contact: Option<SystemTime>) -> StoreStatus {
    StoreStatus { kind, state, since: EPOCH + 50 * SECOND, last_contact, items, dropped: 0, generation: 1 }
}

#[test]
fn the_report_has_one_line_for_each_store_with_its_state_its_count_and_the_age_of_its_data() {
    let refused = CoreError::new(ErrorClass::Forbidden, "secrets is forbidden (HTTP 403)");
    let mut events = store(Kind::Event, StoreState::Live, 20_000, Some(EPOCH + 60 * SECOND));
    events.dropped = 12;
    let ams = ClusterReport {
        name: "ams".to_owned(),
        connections: connections(&[(ChannelState::Live, 3)], ChannelState::Live),
        stores: vec![
            store(Kind::Pod, StoreState::Live, 1204, Some(EPOCH + 58 * SECOND)),
            store(Kind::Secret, StoreState::Forbidden { reason: refused }, 0, None),
            store(Kind::ValidatingAdmissionPolicyBinding, StoreState::Listing { loaded: 500 }, 1, Some(EPOCH)),
            events,
        ],
    };
    // A cluster with no connection: its stores show what they have, with the time.
    let pause = ChannelState::Backoff { failures: 3, pause: SECOND };
    let fra = ClusterReport {
        name: "fra".to_owned(),
        connections: connections(&[(pause, 0), (ChannelState::Retired, 0)], ChannelState::Connecting { attempt: 4 }),
        stores: vec![store(Kind::Pod, StoreState::Waiting(Wait::Channel), 1, Some(EPOCH + 40 * SECOND))],
    };
    let text = render("2026-10-13T14:03:22Z", &[ams, fra], EPOCH + 60 * SECOND);
    let expected = "\
fresnel-cli watch, 2026-10-13T14:03:22Z

cluster ams: 2 of 4 stores live, 21205 objects, 3 connections, 3 watch streams
  Pod                               A     1204  live, last data 2 s ago
  Secret                            B        0  forbidden: secrets is forbidden (HTTP 403) (for 10 s), no data yet
  ValidatingAdmissionPolicyBinding  B        1  list in progress, 500 objects (for 10 s), last data 60 s ago
  Event                             A    20000  live, last data 0 s ago, 12 objects left for the limit

cluster fra: 0 of 1 store live, 1 object, 1 connection, 0 watch streams
  Pod                               A        1  waits for a connection (for 10 s), last data 20 s ago
";
    assert_eq!(text, expected);
}
