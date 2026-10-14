//! The rules for the name of a key and of a namespace: the rules of the server, so that an
//! edit that the server would refuse does not wait for a save to show it.

/// The longest name of a key, the length of a DNS subdomain.
const MAX_KEY: usize = 253;
/// The longest name of a namespace, the length of a DNS label.
const MAX_NAMESPACE: usize = 63;

/// True for a namespace name: a DNS label of lower case letters, digits and `-`, with a
/// letter or a digit at each end.
pub(super) fn is_namespace(name: &str) -> bool {
    let end = |byte: &u8| byte.is_ascii_lowercase() || byte.is_ascii_digit();
    let bytes = name.as_bytes();
    name.len() <= MAX_NAMESPACE
        && bytes.iter().all(|byte| end(byte) || *byte == b'-')
        && bytes.first().is_some_and(end)
        && bytes.last().is_some_and(end)
}

/// Why the server refuses this name of a key of a Secret or a ConfigMap. `None` for a good
/// name. The rule is `IsConfigMapKey` of the server: letters, digits, `-`, `_` and `.`, and
/// not `.`, `..` or a start with `..`, which would be a path out of a mounted folder.
pub(super) fn key_problem(key: &str) -> Option<String> {
    let problem = if key.is_empty() {
        "a key needs a name"
    } else if key.len() > MAX_KEY {
        "a key has 253 characters or fewer"
    } else if !key.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')) {
        "a key has only letters, digits, `-`, `_` and `.`"
    } else if key.starts_with("..") || key == "." {
        "a key is not `.`, and does not start with `..`"
    } else {
        return None;
    };
    Some(format!("`{key}`: {problem}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_has_the_characters_and_the_length_that_the_server_takes() {
        for good in ["password", "tls.crt", ".dockerconfigjson", "a_b-c.d", "x..", "-"] {
            assert_eq!(key_problem(good), None, "{good}");
        }
        assert_eq!(key_problem(&"k".repeat(253)), None);
        let problem = |key: &str| key_problem(key).unwrap_or_default();
        assert_eq!(problem(""), "``: a key needs a name");
        assert_eq!(problem(&"k".repeat(254)).split(": ").nth(1), Some("a key has 253 characters or fewer"));
        assert_eq!(problem("a/b"), "`a/b`: a key has only letters, digits, `-`, `_` and `.`");
        assert_eq!(problem("my key"), "`my key`: a key has only letters, digits, `-`, `_` and `.`");
        assert_eq!(problem("schlüssel"), "`schlüssel`: a key has only letters, digits, `-`, `_` and `.`");
        assert_eq!(problem("."), "`.`: a key is not `.`, and does not start with `..`");
        assert_eq!(problem(".."), "`..`: a key is not `.`, and does not start with `..`");
        assert_eq!(problem("..data"), "`..data`: a key is not `.`, and does not start with `..`");
    }

    #[test]
    fn a_namespace_is_a_dns_label() {
        for good in ["shop", "kube-system", "a", "0-x9", &"n".repeat(63)] {
            assert!(is_namespace(good), "{good}");
        }
        for bad in ["", "Shop", "-shop", "shop-", "a/b", "..", "a.b", "a b", "%2e%2e", &"n".repeat(64)] {
            assert!(!is_namespace(bad), "{bad}");
        }
    }
}
