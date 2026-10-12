//! The kind of a request, and the time limits that follow from it (plan section 8.10).

use std::time::Duration;

/// What a request is. The caller sets it as an extension of the request.
///
/// The layer never reads the kind from the text of a URL: a change of that text would send
/// a watch to the incorrect connection with no error. A request with no kind is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RequestKind {
    /// A read of one object, or some other small read.
    Get,
    /// One page of a list.
    ListPage,
    /// A watch. Its events arrive with no time limit.
    Watch,
    /// A log stream that follows. Its lines arrive with no time limit.
    LogStream,
    /// A write: create, replace, patch or delete.
    Write,
    /// A request that changes its connection to a raw stream: exec, attach or port forward.
    Upgrade,
}

/// The time limits of the requests. Each request has one (design rule 2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Limits {
    /// For a read, with its full answer.
    pub get: Duration,
    /// For the head of the answer to a watch or to a log stream.
    pub stream_start: Duration,
    /// For the head of the answer to an upgrade.
    pub upgrade: Duration,
    /// For one list page: the longest time with no data.
    pub list_idle: Duration,
    /// For one list page, with its full answer.
    pub list_total: Duration,
    /// For a write, with its full answer.
    pub write: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            get: Duration::from_secs(10),
            stream_start: Duration::from_secs(10),
            upgrade: Duration::from_secs(10),
            list_idle: Duration::from_secs(15),
            list_total: Duration::from_secs(120),
            write: Duration::from_secs(15),
        }
    }
}

/// The connections that carry a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lane {
    Requests,
    Watches,
    Logs,
    Upgrade,
}

/// How the layer handles a request of one kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Plan {
    pub(crate) lane: Lane,
    /// The limit for the head of the answer.
    pub(crate) head: Duration,
    /// The longest time between two pieces of the body.
    pub(crate) body_idle: Option<Duration>,
    /// The limit for the full answer, from the start of the request.
    pub(crate) total: Option<Duration>,
    /// Lists and reads ask for gzip. A stream does not: its lines must arrive one by one.
    pub(crate) gzip: bool,
}

impl RequestKind {
    pub(crate) fn plan(self, limits: &Limits) -> Plan {
        let (lane, head, body_idle, total, gzip) = match self {
            Self::Get => (Lane::Requests, limits.get, None, Some(limits.get), true),
            Self::ListPage => (Lane::Requests, limits.list_idle, Some(limits.list_idle), Some(limits.list_total), true),
            Self::Watch => (Lane::Watches, limits.stream_start, None, None, false),
            Self::LogStream => (Lane::Logs, limits.stream_start, None, None, false),
            Self::Write => (Lane::Requests, limits.write, None, Some(limits.write), false),
            Self::Upgrade => (Lane::Upgrade, limits.upgrade, None, None, false),
        };
        Plan { lane, head, body_idle, total, gzip }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: Duration = Duration::from_secs(1);

    #[test]
    fn each_kind_has_the_limits_of_the_plan() {
        let plan = |kind: RequestKind| {
            let plan = kind.plan(&Limits::default());
            (plan.lane, plan.head, plan.body_idle, plan.total, plan.gzip)
        };
        assert_eq!(plan(RequestKind::Get), (Lane::Requests, 10 * SECOND, None, Some(10 * SECOND), true));
        assert_eq!(
            plan(RequestKind::ListPage),
            (Lane::Requests, 15 * SECOND, Some(15 * SECOND), Some(120 * SECOND), true)
        );
        assert_eq!(plan(RequestKind::Watch), (Lane::Watches, 10 * SECOND, None, None, false));
        assert_eq!(plan(RequestKind::LogStream), (Lane::Logs, 10 * SECOND, None, None, false));
        assert_eq!(plan(RequestKind::Write), (Lane::Requests, 15 * SECOND, None, Some(15 * SECOND), false));
        assert_eq!(plan(RequestKind::Upgrade), (Lane::Upgrade, 10 * SECOND, None, None, false));
    }
}
