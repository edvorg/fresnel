//! The tasks of the local connections of one run of a forward, to the limit of the forward
//! (design rules 3 and 12). The connections of a run end with it.

use std::io;
use std::sync::Arc;

use fresnel_net::local::LocalStream;
use tokio::sync::watch;
use tokio::task::{JoinError, JoinSet};

use super::accept::Accepts;
use super::conn;
use super::rule::ForwardRule;
use super::shared::{Gate, Shared};

/// The connections of one run.
pub(super) struct Clients {
    rule: ForwardRule,
    shared: Arc<Shared>,
    tasks: JoinSet<()>,
}

impl Clients {
    pub(super) fn new(rule: ForwardRule, shared: Arc<Shared>) -> Self {
        Self { rule, shared, tasks: JoinSet::new() }
    }

    /// Gives an accepted connection its task, or a reset above the limit. Each connection that
    /// ended leaves the count first: its end and the accept can be ready in one turn of the
    /// task, and when the accept comes first, a client below the limit would get a reset.
    pub(super) fn admit(
        &mut self,
        accepted: io::Result<LocalStream>,
        accepts: &mut Accepts,
        gate: &watch::Sender<Gate>,
    ) {
        while let Some(done) = self.tasks.try_join_next() {
            self.joined(done);
        }
        if let Some(local) = accepts.take(accepted, self.tasks.len(), &self.shared.counts) {
            self.tasks.spawn(conn::serve(local, Arc::clone(&self.shared), gate.subscribe()));
        }
    }

    /// Waits for the end of a connection and takes it. With no connection, the wait has no end.
    pub(super) async fn ended(&mut self) {
        match self.tasks.join_next().await {
            Some(done) => self.joined(done),
            None => std::future::pending().await,
        }
    }

    /// Takes the end of the task of a connection. It ends with no result; its outcome went to
    /// the reports.
    fn joined(&self, done: Result<(), JoinError>) {
        if let Err(error) = done
            && error.is_panic()
        {
            let text = format!("{}: the task of a local connection panicked: {error}", self.rule);
            self.shared.journal.error(self.rule.cluster.as_str(), text);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};
    use std::time::Duration;

    use fresnel_net::WallClock;
    use fresnel_net::local::LocalListener;
    use tokio::net::TcpStream;

    use super::*;
    use crate::cluster::ClusterId;
    use crate::forward::{Bind, LocalPort};
    use crate::journal::Journal;
    use crate::kinds::Kind;
    use crate::store::ObjectKey;

    /// A connection that ended and that the task did not take yet leaves the count before the
    /// check of the limit: at a limit of one, the next client gets its task and no reset.
    #[tokio::test]
    async fn a_connection_that_ended_leaves_the_count_before_the_check_of_the_limit() {
        let rule = ForwardRule {
            cluster: ClusterId::new("ams"),
            kind: Kind::Pod,
            key: ObjectKey::new("shop", "web-1"),
            port: 8080,
            local: LocalPort::Automatic,
            bind: Bind::Localhost,
        };
        let times = [Duration::from_secs(30), Duration::from_secs(1), Duration::from_millis(500)];
        let shared = Arc::new(Shared::new(Journal::new(10, WallClock::system()), times, WallClock::system()));
        let mut clients = Clients::new(rule, Arc::clone(&shared));
        clients.tasks.spawn(async {});
        // The runtime of the test has one thread: the task ends in the turn that this gives it.
        tokio::task::yield_now().await;

        let listener = LocalListener::bind(&[IpAddr::V4(Ipv4Addr::LOCALHOST)], 0).await.expect("a listener");
        let address = *listener.addresses().first().expect("an address");
        let _client = TcpStream::connect(address).await.expect("a connect");
        let accepted = listener.accept().await;
        clients.admit(accepted, &mut Accepts::new(1), &watch::Sender::new(Gate::default()));
        assert_eq!((clients.tasks.len(), shared.counts.read().0.reset), (1, 0), "the new client has its task");
    }
}
