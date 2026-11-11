//! Staying within a deployment's tokens-per-minute (TPM) quota, and waiting out rate limits.
//!
//! Azure admits a request against the deployment's TPM by its prompt plus its output allowance
//! (`max_output_tokens`), whether or not the prompt is cached or chained with
//! `previous_response_id`. When the quota is known (`AZURE_OPENAI_TOKENS_PER_MINUTE`), requests
//! are held locally until the last minute's reservations leave room, instead of being sent to
//! fail. Whatever still comes back as 429 is retried after the delay Azure asks for, for up to
//! [`RATE_LIMIT_PATIENCE`] in total. Both waits are reported, so the UI shows a pause rather than
//! an error.

use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

/// The window Azure's TPM quota is measured over.
pub const WINDOW: Duration = Duration::from_secs(60);
/// The longest a request keeps retrying 429s, waits included, before the error is shown.
pub const RATE_LIMIT_PATIENCE: Duration = Duration::from_secs(5 * 60);
/// The longest single wait accepted from a `retry-after` header or message.
const MAX_RATE_LIMIT_WAIT: Duration = Duration::from_secs(120);

/// Why a request is not sent yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WaitReason {
    /// Held locally: sending now would exceed the configured tokens-per-minute quota.
    Quota,
    /// Azure answered 429; retrying after the delay it asked for.
    RateLimited,
}

/// Reservations of the last minute for one deployment.
#[derive(Debug, Default)]
struct Window {
    entries: VecDeque<(Instant, u64, u64)>,
    next_id: u64,
}

impl Window {
    fn prune(&mut self, now: Instant) {
        while self
            .entries
            .front()
            .is_some_and(|(at, _, _)| now.saturating_duration_since(*at) >= WINDOW)
        {
            self.entries.pop_front();
        }
    }
}

/// How long until `cost` more tokens fit under `limit`, given the reservations of the last
/// minute (oldest first); `None` when it fits now. A request larger than the whole quota is
/// let through once the window is empty: Azure decides about it.
fn wait_needed(
    entries: impl Iterator<Item = (Instant, u64)>,
    limit: u64,
    cost: u64,
    now: Instant,
) -> Option<Duration> {
    let live: Vec<(Instant, u64)> = entries
        .filter(|(at, _)| now.saturating_duration_since(*at) < WINDOW)
        .collect();
    let mut used: u64 = live.iter().map(|(_, n)| n).sum();
    if live.is_empty() || used.saturating_add(cost) <= limit {
        return None;
    }
    for (at, n) in &live {
        used -= n;
        if used.saturating_add(cost) <= limit || used == 0 {
            return Some((*at + WINDOW).saturating_duration_since(now));
        }
    }
    None
}

static WINDOWS: LazyLock<Mutex<HashMap<String, Window>>> = LazyLock::new(Default::default);

/// A reservation in a deployment's window; [`release`] it when Azure refused the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Reservation {
    key: String,
    id: u64,
}

/// Reserves `cost` tokens now, or says how long to wait first.
pub(crate) fn reserve(key: &str, limit: u64, cost: u64) -> Result<Reservation, Duration> {
    let now = Instant::now();
    let mut windows = WINDOWS.lock().unwrap_or_else(|e| e.into_inner());
    let window = windows.entry(key.to_string()).or_default();
    window.prune(now);
    if let Some(wait) = wait_needed(
        window.entries.iter().map(|(at, n, _)| (*at, *n)),
        limit,
        cost,
        now,
    ) {
        // Never a busy loop on a clock edge.
        return Err(wait.max(Duration::from_millis(100)));
    }
    window.next_id += 1;
    let id = window.next_id;
    window.entries.push_back((now, cost, id));
    Ok(Reservation {
        key: key.to_string(),
        id,
    })
}

/// Gives a reservation back: the request was refused and used no quota.
pub(crate) fn release(reservation: &Reservation) {
    let mut windows = WINDOWS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(window) = windows.get_mut(&reservation.key) {
        window.entries.retain(|(_, _, id)| *id != reservation.id);
    }
}

/// The delay a 429 asks for: `retry-after-ms`, `retry-after` (seconds), else Azure's
/// "Please retry after N seconds." in the message. Capped at [`MAX_RATE_LIMIT_WAIT`].
pub(crate) fn rate_limit_delay(headers: &HeaderMap, message: &str) -> Option<Duration> {
    let header = |k: &str| {
        headers
            .get(k)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<f64>().ok())
    };
    let ms = header("retry-after-ms")
        .or_else(|| header("retry-after").map(|s| s * 1000.0))
        .or_else(|| retry_seconds_in(message).map(|s| s * 1000.0))?;
    (ms.is_finite() && ms >= 0.0).then(|| Duration::from_millis(ms as u64).min(MAX_RATE_LIMIT_WAIT))
}

fn retry_seconds_in(message: &str) -> Option<f64> {
    static RE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?i)retry after (\d+(?:\.\d+)?) seconds?").unwrap());
    RE.captures(message)?.get(1)?.as_str().parse().ok()
}

/// A 429 that waiting does not fix: the subscription's quota is used up, not the rate.
pub(crate) fn is_quota_exhausted(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    m.contains("insufficient_quota") || m.contains("exceeded your current quota")
}

/// Backoff for a 429 without a delay: 5 s, 10 s, 20 s, 40 s, then 60 s.
pub(crate) fn rate_limit_backoff(retry: u32) -> Duration {
    Duration::from_secs((5u64 << retry.min(4)).min(60))
}

/// Whole seconds for display, rounded up so a wait never shows as 0 s.
pub fn display_seconds(wait: Duration) -> u64 {
    wait.as_millis().div_ceil(1000) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;

    fn at(base: Instant, secs: u64) -> Instant {
        base + Duration::from_secs(secs)
    }

    #[test]
    fn fits_when_the_window_has_room() {
        let t0 = Instant::now();
        let entries = [(t0, 100_000)];
        assert_eq!(
            wait_needed(entries.into_iter(), 250_000, 140_000, at(t0, 5)),
            None
        );
    }

    #[test]
    fn waits_until_enough_of_the_window_expires() {
        let t0 = Instant::now();
        // 133k at t0 and 50k at t0+20: a further 149k fits only once the first one expires.
        let entries = [(t0, 133_000), (at(t0, 20), 50_000)];
        assert_eq!(
            wait_needed(entries.into_iter(), 250_000, 149_000, at(t0, 10)),
            Some(Duration::from_secs(50))
        );
    }

    #[test]
    fn expired_reservations_do_not_count() {
        let t0 = Instant::now();
        let entries = [(t0, 240_000)];
        assert_eq!(
            wait_needed(entries.into_iter(), 250_000, 100_000, at(t0, 60)),
            None
        );
    }

    #[test]
    fn a_request_larger_than_the_quota_waits_for_an_empty_window_only() {
        let t0 = Instant::now();
        assert_eq!(wait_needed(std::iter::empty(), 250_000, 300_000, t0), None);
        let entries = [(t0, 10_000)];
        assert_eq!(
            wait_needed(entries.into_iter(), 250_000, 300_000, at(t0, 30)),
            Some(Duration::from_secs(30))
        );
    }

    #[test]
    fn reservations_are_per_deployment_and_can_be_released() {
        let key = format!("test|{}", uuid::Uuid::new_v4());
        let first = reserve(&key, 1_000, 800).unwrap();
        let wait = reserve(&key, 1_000, 800).unwrap_err();
        assert!(wait > Duration::from_secs(59) && wait <= WINDOW);
        assert!(reserve(&format!("{key}-other"), 1_000, 800).is_ok());
        release(&first);
        assert!(reserve(&key, 1_000, 800).is_ok());
    }

    #[test]
    fn rate_limit_delay_prefers_headers_then_the_message() {
        let mut h = HeaderMap::new();
        let azure = "Requests to the ChatCompletions_Create Operation have exceeded token rate \
                     limit. Please retry after 47 seconds.";
        assert_eq!(rate_limit_delay(&h, azure), Some(Duration::from_secs(47)));
        h.insert("retry-after", HeaderValue::from_static("12"));
        assert_eq!(rate_limit_delay(&h, azure), Some(Duration::from_secs(12)));
        h.insert("retry-after-ms", HeaderValue::from_static("1500"));
        assert_eq!(
            rate_limit_delay(&h, azure),
            Some(Duration::from_millis(1500))
        );
        h.insert("retry-after-ms", HeaderValue::from_static("900000"));
        assert_eq!(rate_limit_delay(&h, ""), Some(MAX_RATE_LIMIT_WAIT));
        assert_eq!(rate_limit_delay(&HeaderMap::new(), "slow down"), None);
    }

    #[test]
    fn quota_exhaustion_is_not_retried() {
        assert!(is_quota_exhausted(
            "429 You exceeded your current quota, please check your plan and billing details."
        ));
        assert!(!is_quota_exhausted(
            "429 Your requests have exceeded token rate limit."
        ));
    }

    #[test]
    fn backoff_grows_and_caps() {
        let s: Vec<u64> = (0..7).map(|r| rate_limit_backoff(r).as_secs()).collect();
        assert_eq!(s, [5, 10, 20, 40, 60, 60, 60]);
        assert_eq!(display_seconds(Duration::from_millis(100)), 1);
        assert_eq!(display_seconds(Duration::from_secs(47)), 47);
    }
}
