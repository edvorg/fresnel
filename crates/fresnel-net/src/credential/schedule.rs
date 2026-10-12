//! When the next plugin run starts. Each function is free of side effects and cannot panic.

use std::time::{Duration, SystemTime};

use super::CredentialSettings;

/// When to run the plugin again for a token that expires at `expires_at`.
pub(super) fn refresh_time(now: SystemTime, expires_at: SystemTime, settings: &CredentialSettings) -> SystemTime {
    match expires_at.checked_sub(settings.refresh_lead) {
        Some(early) if early > now => early,
        // The token is in its last `refresh_lead` already. A plugin with a cache does this:
        // `doctl` gives its stored token until the expiry time. A run before that time gives
        // the same token, so the next run waits for the expiry. The pause prevents a busy loop.
        Some(_) | None => expires_at.max(later(now, settings.min_refresh_pause)),
    }
}

/// The pause after `failures` failures in sequence: `retry_first`, then two times more for
/// each failure, to `retry_max`.
pub(super) fn retry_pause(failures: u32, settings: &CredentialSettings) -> Duration {
    let doublings = failures.saturating_sub(1).min(31);
    settings.retry_first.saturating_mul(1 << doublings).min(settings.retry_max)
}

/// `now` plus `pause`. A time that the clock cannot hold stays at `now`.
pub(super) fn later(now: SystemTime, pause: Duration) -> SystemTime {
    now.checked_add(pause).unwrap_or(now)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(second: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000 + second)
    }

    #[test]
    fn the_refresh_starts_one_lead_before_the_expiry() {
        let settings = CredentialSettings::default();
        assert_eq!(refresh_time(at(0), at(90), &settings), at(30));
        assert_eq!(refresh_time(at(0), at(7 * 24 * 3600), &settings), at(7 * 24 * 3600 - 60));
    }

    #[test]
    fn a_token_in_its_last_lead_waits_for_its_expiry() {
        let settings = CredentialSettings::default();
        // A plugin with a cache gives the old token again 60 s before the expiry.
        assert_eq!(refresh_time(at(30), at(90), &settings), at(90));
        // The token is expired already: no busy loop.
        assert_eq!(refresh_time(at(95), at(90), &settings), at(105));
        assert_eq!(refresh_time(at(88), at(90), &settings), at(98));
    }

    #[test]
    fn the_retry_pause_doubles_to_the_maximum() {
        let settings = CredentialSettings::default();
        let pauses: Vec<u64> = (1..=7).map(|failures| retry_pause(failures, &settings).as_secs()).collect();
        assert_eq!(pauses, [2, 4, 8, 16, 32, 60, 60]);
        assert_eq!(retry_pause(0, &settings), Duration::from_secs(2));
        assert_eq!(retry_pause(u32::MAX, &settings), Duration::from_secs(60));
    }

    #[test]
    fn extreme_times_do_not_panic() {
        let settings = CredentialSettings { refresh_lead: Duration::MAX, ..CredentialSettings::default() };
        assert_eq!(refresh_time(at(0), at(90), &settings), at(90));
        assert_eq!(later(at(0), Duration::MAX), at(0));
        assert_eq!(refresh_time(at(0), SystemTime::UNIX_EPOCH, &CredentialSettings::default()), at(10));
    }
}
