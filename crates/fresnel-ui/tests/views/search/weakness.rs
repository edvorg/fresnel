//! The weaknesses of Headlamp of plan section 6 that apply to the quick search. Each test
//! has the number of its line in that section.

use egui_kittest::kittest::Queryable as _;
use fresnel_core::CoreError;
use fresnel_core::net::ErrorClass;
use fresnel_core::store::StoreState;

use super::{search, searchable, until_hit};
use crate::fake::{shows, until, view, window};
use crate::kinds::in_panel;

/// Weakness 5: a kind that the search does not cover. Headlamp searches 17 kinds and no
/// Secret. Here the name of a Secret gives a hit, and the hit opens the Secret.
#[test]
fn w5_the_name_of_a_secret_gives_a_hit_that_opens_the_secret() {
    let mut window = window(view(vec![searchable("ams-prod", StoreState::Live)]));
    search(&mut window, "web-secret");
    until_hit(&mut window, "Secret kinds/web-secret of ams-prod");

    window.get_by_label("Secret kinds/web-secret of ams-prod").click();
    until(&mut window, "the panel of the Secret", |window| in_panel(window, "web-secret"));
    assert!(shows(&window, "1 Secret"), "the page of the Secrets is open");
    assert!(window.query_by_label("Search text").is_none(), "the hit closed the search");
}

/// Weakness 6: a search failure with no message. The user cannot list the Secrets of one
/// cluster: the search names that store with the reason, and the hits of the other cluster
/// stay.
#[test]
fn w6_a_cluster_where_secrets_are_forbidden_gives_a_note_and_the_other_hits() {
    let refusal = CoreError::new(ErrorClass::Forbidden, "secrets is forbidden: user \"viewer\" cannot list secrets");
    let clusters = vec![
        searchable("ams-prod", StoreState::Live),
        searchable("fra-stage", StoreState::Forbidden { reason: refusal }),
    ];
    let mut window = window(view(clusters));
    search(&mut window, "web-secret");
    until_hit(&mut window, "Secret kinds/web-secret of ams-prod");
    let note = "Not read: Secrets of fra-stage: forbidden: secrets is forbidden: user \"viewer\" cannot list secrets";
    assert!(shows(&window, note), "the note of the forbidden store");
    assert_eq!(window.query_all_by_label_contains("Secret kinds/web-secret of").count(), 1, "one hit");

    // A text with no hit keeps the note: no hit is no proof of no object.
    window.get_by_label("Search text").type_text("-none");
    until(&mut window, "no hit", |window| shows(window, "0 hits"));
    assert!(shows(&window, note));
}
