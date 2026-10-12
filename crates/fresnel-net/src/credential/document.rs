//! The answer of a credential plugin: an `ExecCredential` document.
//!
//! Limits: the answer must be JSON, and it must hold a token. A plugin that gives a client
//! certificate is not supported.

use std::time::{Duration, SystemTime};

use http::HeaderValue;
use serde_json::Value;

use super::error::PluginFailure;

/// The token of one plugin run, as the value of the `Authorization` header.
#[derive(Clone, Debug)]
pub(crate) struct Credential {
    /// hyper does not print a sensitive value, so `Debug` shows no token.
    pub(crate) header: HeaderValue,
    pub(crate) expires_at: Option<SystemTime>,
}

/// Reads the answer. No error of this function holds a part of the answer: the JSON error
/// gives only its position.
pub(crate) fn parse(stdout: &[u8], now: SystemTime) -> Result<Credential, PluginFailure> {
    let document: Value = serde_json::from_slice(stdout)
        .map_err(|error| PluginFailure::NotJson { line: error.line(), column: error.column() })?;
    let status = document.get("status");
    let token = status.and_then(|status| status.get("token")).and_then(Value::as_str);
    let token = token.filter(|token| !token.is_empty()).ok_or(PluginFailure::NoToken)?;
    let expires_at = match status.and_then(|status| status.get("expirationTimestamp")) {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) => expiry(text, now)?,
        Some(_) => return Err(PluginFailure::BadExpiry),
    };
    let mut header = HeaderValue::try_from(format!("Bearer {token}")).map_err(|_| PluginFailure::BadToken)?;
    header.set_sensitive(true);
    Ok(Credential { header, expires_at })
}

/// The expiry time. A time that is not in the future means "no expiry": a Go program writes
/// its zero time, the year 1, for a token that does not expire. A refresh rule on such a time
/// would run the plugin again and again.
fn expiry(text: &str, now: SystemTime) -> Result<Option<SystemTime>, PluginFailure> {
    let at: jiff::Timestamp = text.parse().map_err(|_| PluginFailure::BadExpiry)?;
    let since_epoch = u64::try_from(at.as_second()).ok().map(Duration::from_secs);
    let at = since_epoch.and_then(|since_epoch| SystemTime::UNIX_EPOCH.checked_add(since_epoch));
    Ok(at.filter(|at| *at > now))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seconds: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
    }

    /// 2026-10-12T20:00:00Z.
    const EXPIRY: u64 = 1_791_835_200;

    fn answer(expiry: &str) -> Vec<u8> {
        format!(r#"{{"kind":"ExecCredential","status":{{"token":"abc","expirationTimestamp":{expiry}}}}}"#).into_bytes()
    }

    #[test]
    fn the_parse_reads_the_token_and_the_expiry() {
        let credential = parse(&answer(r#""2026-10-12T20:00:00Z""#), at(EXPIRY - 90)).expect("a credential");
        assert_eq!(credential.header.as_bytes(), b"Bearer abc");
        assert_eq!(credential.expires_at, Some(at(EXPIRY)));
        assert!(!format!("{credential:?}").contains("abc"), "{credential:?}");
    }

    #[test]
    fn an_expiry_that_is_not_in_the_future_means_no_expiry() {
        for expiry in ["null", r#""0001-01-01T00:00:00Z""#, r#""-009999-01-02T01:59:59Z""#, r#""2026-10-12T20:00:00Z""#]
        {
            let credential = parse(&answer(expiry), at(EXPIRY)).expect("a credential");
            assert_eq!(credential.expires_at, None, "{expiry}");
        }
        let no_field = parse(br#"{"status":{"token":"abc"}}"#, at(0)).expect("a credential");
        assert_eq!(no_field.expires_at, None);
    }

    #[test]
    fn a_bad_answer_gives_its_failure_and_no_part_of_the_answer() {
        let failure = |stdout: &[u8]| parse(stdout, at(0)).expect_err("a failure");
        assert_eq!(failure(br#"{"status":{"token":"secret-1" "#), PluginFailure::NotJson { line: 1, column: 30 });
        assert_eq!(failure(b"apiVersion: v1\nkind: ExecCredential"), PluginFailure::NotJson { line: 1, column: 1 });
        assert_eq!(failure(br#"{"status":"secret-2"}"#), PluginFailure::NoToken);
        assert_eq!(failure(br#"{"status":{"token":""}}"#), PluginFailure::NoToken);
        assert_eq!(failure(br#"{"kind":"ExecCredential"}"#), PluginFailure::NoToken);
        assert_eq!(failure(br#"{"status":{"clientCertificateData":"x","clientKeyData":"y"}}"#), PluginFailure::NoToken);
        assert_eq!(failure(b"{\"status\":{\"token\":\"line1\\nline2\"}}"), PluginFailure::BadToken);
        assert_eq!(failure(&answer(r#""tomorrow""#)), PluginFailure::BadExpiry);
        assert_eq!(failure(&answer("17")), PluginFailure::BadExpiry);
    }
}
