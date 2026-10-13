//! The Journal tab.

use egui_kittest::kittest::Queryable as _;
use fresnel_core::journal::Level;

use crate::fake::{Window, absent, journal, shows, view, window};

fn open_journal(window: &mut Window) {
    window.get_by_label("Journal").click();
    window.run();
}

#[test]
fn the_tab_shows_each_entry_with_its_time_level_and_source_and_filters_by_level() {
    let entries = [
        (Level::Info, "app", "read 4 contexts from 1 kubeconfig file"),
        (Level::Warning, "sgp-prod", "no connection (watches: connect: no answer in 10s); the channel tries again"),
        (Level::Error, "nyc-prod", "no credential (credential plugin `doctl`: stopped with exit status: 1)"),
        (Level::Info, "sgp-prod", "live: a request got an answer on each channel"),
    ];
    let mut with_entries = view(Vec::new());
    with_entries.journal = journal(&entries, 0);
    let mut window = window(with_entries);
    open_journal(&mut window);

    let (texts, sources): (Vec<&str>, Vec<&str>) = entries.iter().map(|(_, source, text)| (*text, *source)).unzip();
    assert_eq!(absent(&window, &texts), Vec::<&str>::new());
    assert_eq!(absent(&window, &sources), Vec::<&str>::new());
    let fixed = ["Time", "Level", "Source", "Entry", "07:59:56", "07:59:59", "info", "warning", "error"];
    assert_eq!(absent(&window, &fixed), Vec::<&str>::new());
    assert!(shows(&window, "1 error, 1 warning"));
    assert!(!window.query_all_by_label_contains("only in the log file").any(|_| true));

    window.get_by_label("Warnings").click();
    window.run();
    assert_eq!(absent(&window, &texts), [texts[0], texts[3]], "warnings and errors stay");
    window.get_by_label("Errors").click();
    window.run();
    assert_eq!(absent(&window, &texts), [texts[0], texts[1], texts[3]]);
    window.get_by_label("All").click();
    window.run();
    assert_eq!(absent(&window, &texts), Vec::<&str>::new());
}

/// Design rule 12: a buffer that drops data shows a counter. The list also draws only the
/// rows on the screen.
#[test]
fn a_full_journal_tells_how_many_entries_left_it_and_draws_only_the_visible_rows() {
    let texts: Vec<String> = (0..10_000).map(|index| format!("entry {index}")).collect();
    let entries: Vec<(Level, &str, &str)> = texts.iter().map(|text| (Level::Info, "app", text.as_str())).collect();
    let mut full = view(Vec::new());
    full.journal = journal(&entries, 2_345);
    let mut window = window(full);
    open_journal(&mut window);

    assert!(shows(&window, "2,345 older entries are only in the log file"));
    assert!(shows(&window, "entry 9999"), "the list starts at its newest entry");
    assert!(!shows(&window, "entry 0"));
    let drawn = window.query_all_by_label_contains("entry ").count();
    assert!((2..40).contains(&drawn), "{drawn} rows are in the frame");

    window.state_mut().view.journal = journal(&entries[..1], 1);
    window.run();
    assert!(shows(&window, "1 older entry is only in the log file"));
}
