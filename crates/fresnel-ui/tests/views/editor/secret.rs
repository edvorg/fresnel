//! The editor of a Secret shows the text values decoded and a binary value as base64 with
//! the tag `!!binary` (plan section 12.9); the check reads the tag, and a hint asks for base64.
//! The save dialog hides the values until the user shows them.

use egui::accesskit::Role;
use egui::{Event, Key};
use egui_kittest::kittest::Queryable as _;
use fresnel_core::app::{EditState, SaveState};
use fresnel_core::kinds::Kind;
use fresnel_core::manifest::Decoded;

use super::save::{answer, saves};
use super::{checked, click_row, editors, give, rows, text};
use crate::fake::{Window, shows, until, view, window};
use crate::keys::scene::{secret, with_secrets};

const DB: &str = "apiVersion: v1\ndata:\n  password: hunter2\n  raw: !!binary /wA=\n  tls.crt: |\n    -----BEGIN \
                  CERTIFICATE-----\n    MIIB\n    -----END CERTIFICATE-----\nkind: Secret\nmetadata:\n  name: db\n  \
                  namespace: scene\ntype: Opaque\n";

/// The editor of the Secret `db`, with its text.
fn editor() -> Window {
    let secrets = [secret("db", "Opaque", &["password"])];
    let mut window = window(view(vec![with_secrets("ams-prod", &secrets)]));
    crate::fake::open_page(&mut window, "Secrets");
    until(&mut window, "the row", |window| shows(window, "db"));
    window.get_by_role_and_label(Role::Label, "db").click_secondary();
    window.run();
    window.get_by_label_contains("Edit manifest…").click();
    window.run();
    let request = editors(&window).pop().expect("the request of the editor");
    assert_eq!(request.target.kind, Kind::Secret);
    give(&mut window, &request, EditState::Done(text(DB, Some(Decoded { texts: 2, binary: 1 }))));
    window
}

#[test]
fn a_secret_shows_its_text_values_decoded_and_a_binary_value_with_its_mark() {
    let mut window = editor();
    let shown = rows(&window);
    for line in ["  password: hunter2", "  raw: !!binary /wA=", "    -----BEGIN CERTIFICATE-----"] {
        assert!(shown.contains(&line.to_owned()), "{line}: {shown:?}");
    }
    let note =
        "The text values of the Secret show decoded. A value that is not text stays base64 with the tag !!binary.";
    assert!(shows(&window, note));
    checked(&mut window, "No error");

    // A binary value that is no longer base64 is a hint: the parser reads it as its text, and
    // the server refuses it in a Secret.
    click_row(&mut window, "  raw: !!binary /wA=");
    window.key_press(Key::Backspace);
    window.run();
    checked(&mut window, "No error, 1 hint");
    let hint = "Line 4, column 17: The value with the tag !!binary is not base64. A Secret takes no such value.";
    assert!(shows(&window, hint));
}

/// The save dialog of a Secret shows its changed keys with dots, and the values only after
/// a click on "Show the values".
#[test]
fn the_save_dialog_of_a_secret_hides_the_values_until_the_user_shows_them() {
    let mut window = editor();
    checked(&mut window, "No error");
    click_row(&mut window, "  password: hunter2");
    window.event(Event::Text("3".to_owned()));
    window.run();
    window.get_by_label("Save the text").click();
    window.run();
    let check = saves(&window).pop().expect("the dry run");
    let after = text(&DB.replace("hunter2", "hunter23"), Some(Decoded { texts: 2, binary: 1 }));
    answer(&mut window, &check, SaveState::Checked(after));
    let (old, new) = ("    3       -   password: hunter2", "          3 +   password: hunter23");
    assert!(
        shows(&window, "    3       -   password: ••••••••") && shows(&window, "          3 +   password: ••••••••")
    );
    assert!(shows(&window, "    5     5     tls.crt: ••••••••"), "a value in the lines around the change");
    assert!(!shows(&window, old) && !shows(&window, new));

    window.get_by_label("Show the values").click();
    window.run();
    assert!(shows(&window, old) && shows(&window, new) && shows(&window, "    5     5     tls.crt: |"));
}
