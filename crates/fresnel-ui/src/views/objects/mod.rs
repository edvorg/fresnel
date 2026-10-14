//! The page of the objects of one kind: one table with the objects of each selected
//! cluster, and the detail panel of one of them (plan sections 6 and 11). A row carries
//! the identity of its cluster, and a row from a store that is not live has a freshness mark.
//!
//! - `model`: what a kind gives to its page. `page`: the page with its table and panel.
//! - `follow`: the changes of the stores from frame to frame. `filter`: the filters.
//! - `toolbar`: the count, the filters and the columns. `notes`: the state of each store.

mod filter;
mod follow;
mod model;
mod notes;
mod page;
mod toolbar;

pub(crate) use filter::{NamespaceSet, Namespaces, ObjectFilter};
pub(crate) use model::{ClusterMark, Nouns, ObjectModel, RowScene, Stores, uid};
pub(crate) use notes::old_since;
pub(crate) use page::ObjectPage;

use egui::Ui;
use fresnel_core::cluster::ClusterId;
use fresnel_core::kinds::Kind;
use fresnel_core::store::ObjectKey;

use crate::shell::Scene;

/// An object that the user wants to see: a link of the detail panel names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Link {
    pub(crate) cluster: ClusterId,
    pub(crate) kind: Kind,
    pub(crate) key: ObjectKey,
}

/// The page of the center of the window for one kind, as the shell holds it: with no
/// type of its objects.
pub(crate) trait Page {
    fn kind(&self) -> Kind;

    /// Draws the page. Gives the object of a link that the user clicked.
    fn show(&mut self, ui: &mut Ui, scene: &mut Scene<'_>) -> Option<Link>;

    /// Gives up the rows, the snapshots, the selection and the panel, for a page that is
    /// not painted (plan section 7.3).
    fn rest(&mut self);

    /// Selects the object of a link of this kind and opens its panel, when the store of
    /// its cluster has it.
    fn open(&mut self, link: Link);
}
