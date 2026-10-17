use std::time::SystemTime;

use fresnel_core::app::ForwardRow;
use fresnel_core::forward::ForwardView;
use fresnel_core::net::task::TaskReport;
use fresnel_core::published::{Disposal, Notice, Publisher};
use tokio::net::TcpListener;
use tokio::runtime::Handle;

use super::*;

fn target() -> Target {
    Target { namespace: "soak".to_owned(), name: "soak-echo".to_owned(), port: 80 }
}

/// A copy of the core with the forward of the soak in this state on this address.
fn look(state: ForwardState, address: SocketAddr, disposal: &Disposal) -> Arc<Look> {
    let rule = target().rule(&ClusterId::new("k3s"));
    let view = ForwardView { addresses: vec![address], ..ForwardView::new(rule, state, SystemTime::UNIX_EPOCH) };
    let row = ForwardRow { id: 1, saved: false, view: Publisher::new(view, disposal.bin(), Notice::new()).read() };
    Arc::new(Look { at: Instant::now(), row: None, journal: Arc::default(), forwards: vec![row].into() })
}

#[test]
fn the_target_reads_its_option() {
    assert_eq!(Target::parse("soak/soak-echo:80"), Ok(target()));
    let text = "the value of --forward is <namespace>/<service>:<port>, not `soak-echo:80`";
    assert_eq!(Target::parse("soak-echo:80"), Err(text.to_owned()));
    assert!(Target::parse("soak/soak-echo:0").is_err() && Target::parse("soak/:80").is_err());
}

/// A page through the local port passes; a port that refuses the connect while the forward
/// shows "active" is a gap with no visible state and a closed port. Windows refuses a connect
/// to a closed port after about 2 s, so a try has more time.
#[tokio::test]
async fn a_check_fetches_the_page_and_a_closed_port_while_active_fails_with_no_visible_state() {
    let limit = Duration::from_secs(4);
    let tasks = Tasks::new(Handle::current(), |report: TaskReport| panic!("{report}"));
    let disposal = Disposal::start(&tasks);
    let server = TcpListener::bind("127.0.0.1:0").await.expect("a listener");
    let address = server.local_addr().expect("its address");
    let serve = async {
        let (mut stream, _) = server.accept().await.expect("a client");
        let mut request = [0_u8; 64];
        let read = stream.read(&mut request).await.expect("the request");
        assert!(read > 0);
        stream.write_all(b"HTTP/1.0 200 OK\r\n\r\nsoak-echo\n").await.expect("the page");
    };
    let (_, looks) = watch::channel(look(ForwardState::Active, address, &disposal));
    let rule = target().rule(&ClusterId::new("k3s"));
    let ((), good) = tokio::join!(serve, check(&rule, &looks, limit));
    assert!(good.ok && good.error.is_none() && !good.without_state, "{good:?}");

    drop(server);
    let bad = check(&rule, &looks, limit).await;
    assert!(!bad.ok && bad.without_state && bad.port_closed, "{bad:?}");
    assert!(bad.seconds >= limit.as_secs_f64(), "the check tried for its limit");
}

#[test]
fn the_verdict_fails_a_gap_with_no_state_a_closed_port_and_a_forward_that_does_not_recover() {
    let check = |ok, without_state, port_closed| Check {
        ok,
        seconds: 1.0,
        state: "active".to_owned(),
        error: (!ok).then(|| "connect: refused".to_owned()),
        without_state,
        port_closed,
    };
    let minute = |checks| Minute { state: "active".to_owned(), connections: String::new(), checks };
    let good = minute(vec![check(true, false, false)]);
    let visible = minute(vec![check(false, false, false)]);
    let (verdict, reasons) = Verdict::of(&[Some(&good), Some(&visible), Some(&good), None], 5);
    assert_eq!((verdict.checks, verdict.failed, verdict.quiet_minutes), (3, 1, 1));
    assert_eq!(reasons, Vec::<String>::new(), "a failure with a visible state that recovers passes");

    let gap = minute(vec![check(false, true, true)]);
    let (_, reasons) = Verdict::of(&[Some(&gap), Some(&visible), Some(&visible), Some(&visible), Some(&visible)], 5);
    assert_eq!(
        reasons,
        [
            "1 checks of the forward: \"active\" and no page in 20 s",
            "1 checks of the forward found its local port closed",
            "the forward gave no page for 5 minutes in a row",
        ]
    );
}
