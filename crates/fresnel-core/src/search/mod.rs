//! The quick search (plan section 12.1): each kind of each selected cluster, in the stores
//! of the app only. It sends no request.
//!
//! A search reads the store of each kind that the cluster has: the full objects of a kind
//! of tier A or of a kind that a view shows, and else the metadata of a kind of tier B.
//! Each hit carries its cluster and its kind, and the mark of a store that is not live
//! (design rule 4). Each store that the search did not read, or not in full, is in the
//! result with the reason, so a forbidden kind never looks like a kind with no match
//! (weaknesses 5 and 6 of plan section 6).
//!
//! The UI runs a search on a worker thread. A run reads `cancel` between two parts.
//!
//! - `source`: the store of one kind as the search reads it. `score`: the match of one object.

mod score;
mod source;

use std::cmp::Reverse;
use std::collections::BTreeSet;
use std::fmt::{self, Display};
use std::time::SystemTime;

pub(crate) use source::Searched;
pub use source::{ClusterSources, Source};

use crate::cluster::ClusterId;
use crate::kinds::Kind;
use crate::store::{ObjectKey, StoreState};
use score::Matcher;

/// The most hits that a result holds. The total count tells how many more match.
pub const MAX_HITS: usize = 200;

/// What the user looks for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    /// Words, each of which matches the name or an other text of an object.
    pub text: String,
    /// The clusters to read. Empty: each selected cluster.
    pub clusters: BTreeSet<ClusterId>,
    /// The kinds to read. Empty: each kind.
    pub kinds: BTreeSet<Kind>,
}

/// One object that matches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub cluster: ClusterId,
    pub kind: Kind,
    pub key: ObjectKey,
    /// Set for an object of a store that is not live: its values can be old.
    pub stale: Option<Stale>,
}

/// The mark of a hit of a store that is not live.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stale {
    /// The last data of the server for the store. `None` before its first answer.
    pub last_contact: Option<SystemTime>,
}

/// A store that the search did not read, or not in full.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unread {
    pub cluster: ClusterId,
    /// `None` for each kind of a cluster with no stores.
    pub kind: Option<Kind>,
    pub why: Why,
}

/// Why the search did not read a store, or not in full.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Why {
    /// The cluster has no stores, for this reason.
    NoStores(String),
    /// The app holds no store of the kind now: a kind of tier C, or the Events of the
    /// setting "on demand", while no page shows it.
    NoStore,
    /// The server refuses the kind: the store is forbidden or not supported.
    Refused(StoreState),
    /// The store has no complete list. The search read the objects that it has.
    Incomplete(StoreState),
}

impl Display for Why {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoStores(reason) => write!(f, "no store: {reason}"),
            Self::NoStore => f.write_str("the app holds its objects only while a page shows them"),
            Self::Refused(state) => Display::fmt(state, f),
            Self::Incomplete(state) => write!(f, "its first list is not complete ({state}); the search read a part"),
        }
    }
}

/// The result of one search.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    /// The best hits, the best first: at most [`MAX_HITS`].
    pub hits: Vec<Hit>,
    /// The count of all objects that match.
    pub total: usize,
    /// Each store of the query that the search did not read, or not in full.
    pub unread: Vec<Unread>,
}

/// One object that matches, before the choice of the best ones: its score, its kind, its
/// identity, its cluster and the mark of its store.
type Match<'a> = (Reverse<u32>, Kind, &'a ObjectKey, &'a ClusterId, Option<Stale>);

/// Searches the stores of the selected clusters. `None` when `cancel` stopped the search.
pub fn run(query: &Query, clusters: &[ClusterSources], cancel: &dyn Fn() -> bool) -> Option<Found> {
    let matcher = Matcher::new(&query.text);
    let wanted_kinds = || Kind::ALL.iter().copied().filter(|kind| query.kinds.is_empty() || query.kinds.contains(kind));
    let mut found = Found::default();
    let mut matches: Vec<Match<'_>> = Vec::new();
    for cluster in
        clusters.iter().filter(|cluster| query.clusters.is_empty() || query.clusters.contains(&cluster.cluster))
    {
        let id = &cluster.cluster;
        let sources = match &cluster.sources {
            Ok(sources) => sources,
            Err(reason) => {
                found.unread.push(Unread { cluster: id.clone(), kind: None, why: Why::NoStores(reason.clone()) });
                continue;
            }
        };
        for kind in wanted_kinds() {
            let Some(source) = sources.iter().find(|source| source.status.kind == kind) else {
                found.unread.push(Unread { cluster: id.clone(), kind: Some(kind), why: Why::NoStore });
                continue;
            };
            let state = &source.status.state;
            let why = match state {
                StoreState::Forbidden { .. } | StoreState::NotSupported { .. } => Some(Why::Refused(state.clone())),
                _ if !source.status.listed => Some(Why::Incomplete(state.clone())),
                _ => None,
            };
            found.unread.extend(why.map(|why| Unread { cluster: id.clone(), kind: Some(kind), why }));
            if matcher.is_empty() {
                continue;
            }
            let stale = (!state.is_live()).then_some(Stale { last_contact: source.status.last_contact });
            let complete = source.find(&matcher, cancel, &mut |key, score| {
                matches.push((Reverse(score), kind, key, id, stale));
            });
            if !complete {
                return None;
            }
        }
    }
    found.total = matches.len();
    // The best hits by score; equal scores in the sequence of the kinds, then by name.
    let order = |a: &Match<'_>, b: &Match<'_>| {
        (a.0, a.1, &a.2.name, &a.2.namespace, a.3).cmp(&(b.0, b.1, &b.2.name, &b.2.namespace, b.3))
    };
    if matches.len() > MAX_HITS {
        matches.select_nth_unstable_by(MAX_HITS, order);
        matches.truncate(MAX_HITS);
    }
    matches.sort_unstable_by(order);
    found.hits = matches
        .into_iter()
        .map(|(_, kind, key, cluster, stale)| Hit { cluster: cluster.clone(), kind, key: key.clone(), stale })
        .collect();
    Some(found)
}

#[cfg(test)]
mod tests;
