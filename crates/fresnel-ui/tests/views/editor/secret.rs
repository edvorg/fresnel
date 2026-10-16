//! The editor of a Secret shows the text values decoded and a binary value as base64 with
//! the tag `!!binary` (plan section 12.9); the check reads the tag and asks for base64.

use egui::Key;
use egui::accesskit::Role;
use egui_kittest::kittest::Queryable as _;
use fresnel_core::app::EditState;
use fresnel_core::kinds::Kind;
use fresnel_core::manifest::Decoded;

use super::{checked, click_row, editors, give, rows, text};
use crate::fake::{shows, until, view, window};
use crate::keys::scene::{secret, with_secrets};

const DB: &str = "apiVersion: v1\ndata:\n  password: hunter2\n  raw: !!binary /wA=\n  tls.crt: |\n    -----BEGIN \
                  CERTIFICATE-----\n    MIIB\n    -----END CERTIFICATE-----\nkind: Secret\nmetadata:\n  name: db\n  \
                  namespace: scene\ntype: Opaque\n";

#[test]
fn a_secret_shows_its_text_values_decoded_and_a_binary_value_with_its_mark() {
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
    let shown = rows(&window);
    for line in ["  password: hunter2", "  raw: !!binary /wA=", "    -----BEGIN CERTIFICATE-----"] {
        assert!(shown.contains(&line.to_owned()), "{line}: {shown:?}");
    }
    let note =
        "The text values of the Secret show decoded. A value that is not text stays base64 with the tag !!binary.";
    assert!(shows(&window, note));
    checked(&mut window, "No error");

    // A binary value that is no longer base64 is an error of the YAML type.
    click_row(&mut window, "  raw: !!binary /wA=");
    window.key_press(Key::Backspace);
    window.run();
    checked(&mut window, "1 error");
    assert!(shows(&window, "Line 4, column 17: The value with the tag !!binary is not base64."));
}
