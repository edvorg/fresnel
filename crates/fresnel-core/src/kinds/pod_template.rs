//! The PodTemplate: its description.

use k8s_openapi::api::core::v1::{PodSpec, PodTemplate};

use super::describe::by_metadata;
use super::health::ObjectHealth;
use super::{Action, Column, Describe, Kind, Sidebar, Tier, read};

impl Describe for PodTemplate {
    const ID: Kind = Kind::PodTemplate;
    const TITLE: &'static str = "PodTemplates";
    const SIDEBAR: Sidebar = Sidebar::Other;
    const TIER: Tier = Tier::B;
    const COLUMNS: &'static [Column<Self>] = &[
        Column::text("Containers", 140.0, |template| read::containers(pod_spec(template))),
        Column::text("Images", 220.0, |template| read::images(pod_spec(template))),
        Column::text("Pod labels", 180.0, |template| {
            read::labels(template.template.as_ref().and_then(|template| template.metadata.as_ref()?.labels.as_ref()))
        }),
    ];
    const ACTIONS: &'static [Action] = &[];

    fn health(&self) -> ObjectHealth {
        by_metadata(&self.metadata)
    }

    fn search<'a>(&'a self, texts: &mut Vec<&'a str>) {
        read::search_images(pod_spec(self), texts);
    }
}

fn pod_spec(template: &PodTemplate) -> Option<&PodSpec> {
    read::pod_spec(template.template.as_ref())
}
