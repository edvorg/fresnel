//! The registries of an image pull Secret (research 08 section 8.4): each registry with its
//! user. The password stays in the value of the key, and no type of this module holds it.

use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::Deserialize;

/// One registry of an image pull Secret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registry {
    /// The address of the registry, as the Secret names it.
    pub server: String,
    /// The user name: the field `username`, or the part of `auth` before its colon.
    pub username: Option<String>,
    /// The e-mail address, which some old clients sent.
    pub email: Option<String>,
    /// True when the entry has a password or a token. A view shows this fact only.
    pub has_password: bool,
}

/// The form of the JSON.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Format {
    /// `.dockerconfigjson`: `{"auths": {"<registry>": {...}}}`.
    ConfigJson,
    /// `.dockercfg`, the older form: `{"<registry>": {...}}`.
    Cfg,
}

/// The JSON of `.dockerconfigjson`.
#[derive(Deserialize)]
struct ConfigJson {
    #[serde(default)]
    auths: BTreeMap<String, Login>,
}

/// The login of one registry. It has no `Debug`: it holds the password.
#[derive(Default, Deserialize)]
#[serde(default)]
struct Login {
    username: Option<String>,
    password: Option<String>,
    email: Option<String>,
    /// The base64 of `user:password`.
    auth: Option<String>,
    identitytoken: Option<String>,
    registrytoken: Option<String>,
}

/// The registries of the JSON of an image pull Secret, in the sequence of their names.
/// The reason of a JSON that does not read has its place only: the text of the JSON
/// reader can hold a part of a password.
pub(super) fn registries(json: &[u8], format: Format) -> Result<Vec<Registry>, String> {
    let position = |error: serde_json::Error| {
        format!("the JSON does not read: an error at line {}, column {}", error.line(), error.column())
    };
    let logins = match format {
        Format::ConfigJson => serde_json::from_slice::<ConfigJson>(json).map_err(position)?.auths,
        Format::Cfg => serde_json::from_slice::<BTreeMap<String, Login>>(json).map_err(position)?,
    };
    Ok(logins.into_iter().map(|(server, login)| registry(server, login)).collect())
}

fn registry(server: String, login: Login) -> Registry {
    let pair = login.auth.as_deref().and_then(|auth| STANDARD.decode(auth.trim()).ok());
    let pair = pair.as_deref().and_then(|pair| std::str::from_utf8(pair).ok()).and_then(|pair| pair.split_once(':'));
    let filled = |field: &Option<String>| field.as_deref().is_some_and(|text| !text.is_empty());
    let has_password = filled(&login.password)
        || pair.is_some_and(|(_, password)| !password.is_empty())
        || filled(&login.identitytoken)
        || filled(&login.registrytoken);
    let username = login.username.filter(|name| !name.is_empty()).or_else(|| pair.map(|(name, _)| name.to_owned()));
    Registry { server, username: username.filter(|name| !name.is_empty()), email: login.email, has_password }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry(server: &str, username: Option<&str>, has_password: bool) -> Registry {
        Registry { server: server.to_owned(), username: username.map(str::to_owned), email: None, has_password }
    }

    #[test]
    fn each_registry_gives_its_user_from_the_name_field_or_from_auth_and_no_password() {
        // "robot:s3cr3t" and ":only-a-password" in base64.
        let json = br#"{"auths":{
            "ghcr.io":{"username":"octo","password":"s3cr3t","email":"octo@example.com"},
            "registry.example.com":{"auth":"cm9ib3Q6czNjcjN0"},
            "token.example.com":{"auth":"Om9ubHktYS1wYXNzd29yZA=="},
            "id.example.com":{"identitytoken":"t0ken"},
            "empty.example.com":{}}}"#;
        let found = registries(json, Format::ConfigJson).expect("the registries");
        let mut ghcr = registry("ghcr.io", Some("octo"), true);
        ghcr.email = Some("octo@example.com".to_owned());
        assert_eq!(
            found,
            [
                registry("empty.example.com", None, false),
                ghcr,
                registry("id.example.com", None, true),
                registry("registry.example.com", Some("robot"), true),
                registry("token.example.com", None, true),
            ]
        );
        let debug = format!("{found:?}");
        assert!(!debug.contains("s3cr3t") && !debug.contains("t0ken") && !debug.contains("cm9ib3Q"), "{debug}");
    }

    #[test]
    fn the_older_form_is_the_map_itself_and_a_broken_json_gives_its_place_and_no_value() {
        let cfg = br#"{"quay.io":{"auth":"cm9ib3Q6czNjcjN0","email":"a@b"}}"#;
        let found = registries(cfg, Format::Cfg).expect("the registries");
        assert_eq!(
            found.iter().map(|r| (r.server.as_str(), r.username.as_deref())).collect::<Vec<_>>(),
            [("quay.io", Some("robot"))]
        );
        assert_eq!(registries(br#"{"other":1}"#, Format::ConfigJson), Ok(Vec::new()), "no `auths`");
        assert_eq!(registries(b"{}", Format::Cfg), Ok(Vec::new()));
        let bad = registries(b"{\"auths\":{\"x\":{\"password\":s3cr3t}}}", Format::ConfigJson);
        assert_eq!(bad, Err("the JSON does not read: an error at line 1, column 27".to_owned()));
        let wrong_type = registries(br#"{"auths":{"x":{"password":["s3cr3t"]}}}"#, Format::ConfigJson);
        assert!(wrong_type.as_ref().is_err_and(|reason| !reason.contains("s3cr3t")), "{wrong_type:?}");
        assert!(registries(b"\xff", Format::Cfg).is_err(), "bytes that are not JSON");
    }
}
