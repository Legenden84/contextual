//! Hand-written Azure AI Foundry HTTP client (no OpenAI SDK), reproducing the requests the
//! `openai` npm SDK made for the old server:
//!
//! - Chat completions (`AzureOpenAI`): `POST <endpoint>/openai/deployments/<deployment>/chat/completions?api-version=<v>`
//!   with header `api-key`.
//! - Responses API (`OpenAI` on Azure's v1 base): `POST <endpoint>/openai/v1/responses`, no
//!   api-version, with `Authorization: Bearer <key>` and `api-key`.
//!
//! Like the SDK it retries twice on connection errors, 408, 409 and 5xx (honouring
//! `retry-after-ms` / `retry-after` / `x-should-retry`), never follows redirects (the key would
//! leak), and builds error messages the same way (`"<status> <error.message>"`). Rate limits
//! (429) are waited out for longer, and requests are held back to the deployment's
//! tokens-per-minute quota when it is configured; see [`super::pacing`].

use super::config::FoundryConfig;
use super::pacing::{self, Reservation, WaitReason};
use super::types::ChatEvent;
use crate::error::{Error, ErrorCode};
use crate::logic::context::{BudgetExceeded, FALLBACK_CONTEXT_LIMIT, RequestBudget};
use crate::models;
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// The SDK's default `maxRetries`.
pub const DEFAULT_MAX_RETRIES: u32 = 2;
/// The SDK's default timeout (until response headers arrive).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);

/// A failed Foundry request, classified like the SDK's error classes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FoundryError {
    /// HTTP error status; `message` is the SDK-style `"<status> <error.message>"`.
    Status { status: u16, message: String },
    /// Could not connect or the connection broke before a response (`"Connection error."`).
    Connection(String),
    /// No response headers within the timeout (`"Request timed out."`).
    Timeout,
    /// Error without a status: in-stream errors, malformed responses, cut-off output.
    Failed(String),
    /// Refused locally before any HTTP request, retry or API fallback.
    ContextBudget(BudgetExceeded),
    /// Refused locally because the request itself cannot be sent as it is, for example a
    /// transcript whose calls and results do not match. Nothing reached the provider.
    Rejected(String),
    /// Cancelled by the caller.
    Cancelled,
}

impl FoundryError {
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Status { status, .. } => Some(*status),
            _ => None,
        }
    }

    /// The error's own message (the SDK's `err.message`).
    pub fn raw_message(&self) -> String {
        match self {
            Self::Status { message, .. } => message.clone(),
            Self::Connection(m) | Self::Failed(m) | Self::Rejected(m) => m.clone(),
            Self::Timeout => "Request timed out.".into(),
            Self::ContextBudget(e) => e.to_string(),
            Self::Cancelled => "Request was aborted.".into(),
        }
    }
}

/// A readable message for HTTP and network errors, without echoing secrets
/// (port of `describeError`): `(status, message)`.
pub fn describe_error(err: &FoundryError) -> (u16, String) {
    match err {
        FoundryError::Status { status, message } => {
            (*status, format!("Foundry returned {status}: {message}"))
        }
        FoundryError::ContextBudget(e) => (400, e.to_string()),
        FoundryError::Rejected(message) => (400, message.clone()),
        other => (
            502,
            format!("Could not reach Foundry: {}", other.raw_message()),
        ),
    }
}

impl From<FoundryError> for Error {
    fn from(e: FoundryError) -> Self {
        let code = match &e {
            FoundryError::Cancelled => ErrorCode::Cancelled,
            FoundryError::ContextBudget(_) | FoundryError::Rejected(_) => ErrorCode::InvalidInput,
            _ => ErrorCode::Upstream,
        };
        Error::new(code, describe_error(&e).1)
    }
}

/// The SDK's `APIError.makeMessage` for an error response body.
pub(crate) fn status_message(status: u16, body: &str) -> String {
    let msg = match serde_json::from_str::<serde_json::Value>(body) {
        Ok(json) => json
            .get("error")
            .filter(|e| truthy(e))
            .map(error_payload_message),
        Err(_) => (!body.is_empty()).then(|| body.to_string()),
    };
    match msg {
        Some(m) if !m.is_empty() => format!("{status} {m}"),
        _ => format!("{status} status code (no body)"),
    }
}

/// `error.message` when it's a string, else the JSON of the message or of the whole error.
pub(crate) fn error_payload_message(err: &serde_json::Value) -> String {
    match err.get("message").filter(|m| truthy(m)) {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
        None => err.to_string(),
    }
}

/// JavaScript truthiness of a JSON value.
pub(crate) fn truthy(v: &serde_json::Value) -> bool {
    match v {
        serde_json::Value::Null => false,
        serde_json::Value::Bool(b) => *b,
        serde_json::Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        serde_json::Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

/// Whether each endpoint+deployment supports the Responses API, learned on first use
/// (process-wide, like the old module-level map).
static RESPONSES_SUPPORT: LazyLock<Mutex<HashMap<String, bool>>> = LazyLock::new(Default::default);
static TOOLS_SUPPORT: LazyLock<Mutex<HashMap<String, bool>>> = LazyLock::new(Default::default);

/// What a request costs against the deployment's tokens-per-minute quota (prompt plus output
/// allowance, as Azure counts it), and where waits are reported.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Pace<'a> {
    pub cost: u64,
    pub notify: Option<&'a mpsc::Sender<ChatEvent>>,
}

impl Pace<'_> {
    /// Reports a wait of a second or more; a full queue only loses a status line.
    fn report(&self, wait: Duration, reason: WaitReason) {
        if let Some(tx) = self.notify.filter(|_| wait >= Duration::from_secs(1)) {
            let _ = tx.try_send(ChatEvent::Waiting {
                seconds: pacing::display_seconds(wait),
                reason,
            });
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Api {
    Chat,
    Responses,
}

/// Client for one Foundry configuration. Cheap to clone.
#[derive(Debug, Clone)]
pub struct FoundryClient {
    pub(crate) http: reqwest::Client,
    pub(crate) cfg: FoundryConfig,
    max_retries: u32,
    timeout: Duration,
}

impl FoundryClient {
    pub fn new(cfg: FoundryConfig) -> crate::Result<Self> {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("contextual/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| Error::internal(format!("HTTP client: {e}")))?;
        Ok(Self {
            http,
            cfg,
            max_retries: DEFAULT_MAX_RETRIES,
            timeout: DEFAULT_TIMEOUT,
        })
    }

    /// Retries on connection errors, 408/409/429/5xx (default 2, like the SDK).
    pub fn with_max_retries(mut self, n: u32) -> Self {
        self.max_retries = n;
        self
    }

    /// Time allowed until response headers arrive (default 10 minutes, like the SDK).
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn config(&self) -> &FoundryConfig {
        &self.cfg
    }

    /// Budget for a request's actual output allowance (drafts and connection tests use less).
    /// The app supplies its resolved limit, including custom catalog entries. Standalone
    /// clients fall back to the built-in catalog, then the default context limit.
    pub fn request_budget(&self, output_tokens: u32) -> RequestBudget {
        let limit = self
            .cfg
            .context_limit
            .or_else(|| {
                models::find(&[], &[&self.cfg.model_name, &self.cfg.deployment])
                    .map(|model| model.context_limit)
            })
            .unwrap_or(FALLBACK_CONTEXT_LIMIT);
        RequestBudget::new(limit, u64::from(output_tokens))
    }

    pub(crate) fn check_input_budget(
        &self,
        input_tokens: u64,
        output_tokens: u32,
    ) -> Result<(), FoundryError> {
        self.request_budget(output_tokens)
            .check(input_tokens)
            .map_err(FoundryError::ContextBudget)
    }

    fn support_key(&self) -> String {
        format!("{}|{}", self.cfg.endpoint, self.cfg.deployment)
    }

    /// What has been learned about the Responses API for this endpoint+deployment.
    pub fn responses_supported(&self) -> Option<bool> {
        RESPONSES_SUPPORT
            .lock()
            .ok()?
            .get(&self.support_key())
            .copied()
    }

    pub(crate) fn set_responses_supported(&self, value: bool) {
        if let Ok(mut m) = RESPONSES_SUPPORT.lock() {
            m.insert(self.support_key(), value);
        }
    }

    /// What has been learned about tool declarations for this endpoint+deployment. This is
    /// tracked separately from Responses API support: one does not imply the other.
    pub fn tools_supported(&self) -> Option<bool> {
        TOOLS_SUPPORT.lock().ok()?.get(&self.support_key()).copied()
    }

    pub(crate) fn set_tools_supported(&self, value: bool) {
        if let Ok(mut m) = TOOLS_SUPPORT.lock() {
            m.insert(self.support_key(), value);
        }
    }

    /// `<endpoint>/openai/v1/responses`.
    pub fn responses_url(&self) -> String {
        let e = &self.cfg.endpoint;
        let base = if e.ends_with('/') {
            e.clone()
        } else {
            format!("{e}/")
        };
        format!("{base}openai/v1/responses")
    }

    /// `<endpoint>/openai/deployments/<deployment>/chat/completions?api-version=<v>`.
    pub fn chat_url(&self) -> crate::Result<reqwest::Url> {
        let base = format!("{}/openai", self.cfg.endpoint.trim_end_matches('/'));
        let mut url = reqwest::Url::parse(&base)
            .map_err(|e| Error::invalid(format!("Invalid endpoint: {e}")))?;
        let has_deployments = url.path().split('/').any(|s| s == "deployments");
        {
            let mut segs = url
                .path_segments_mut()
                .map_err(|_| Error::invalid("Invalid endpoint"))?;
            segs.pop_if_empty();
            if !has_deployments {
                segs.push("deployments").push(&self.cfg.deployment);
            }
            segs.push("chat").push("completions");
        }
        url.query_pairs_mut()
            .append_pair("api-version", &self.cfg.api_version);
        Ok(url)
    }

    fn headers(&self, api: Api) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        h.insert(ACCEPT, HeaderValue::from_static("application/json"));
        if let Ok(mut key) = HeaderValue::from_str(&self.cfg.api_key) {
            key.set_sensitive(true);
            h.insert("api-key", key);
        }
        if let Api::Responses = api {
            if let Ok(mut bearer) = HeaderValue::from_str(&format!("Bearer {}", self.cfg.api_key)) {
                bearer.set_sensitive(true);
                h.insert(AUTHORIZATION, bearer);
            }
        }
        h
    }

    /// Holds the request until it fits the configured tokens-per-minute quota, then reserves
    /// its cost. Without a quota (or a cost) nothing is held.
    async fn admit(
        &self,
        pace: Pace<'_>,
        cancel: &CancellationToken,
    ) -> Result<Option<Reservation>, FoundryError> {
        let Some(limit) = self.cfg.tokens_per_minute.filter(|_| pace.cost > 0) else {
            return Ok(None);
        };
        let key = self.support_key();
        loop {
            match pacing::reserve(&key, limit, pace.cost) {
                Ok(reservation) => return Ok(Some(reservation)),
                Err(wait) => {
                    tracing::info!(
                        "Holding a request of ~{} tokens for {wait:?} (quota {limit}/min)",
                        pace.cost
                    );
                    pace.report(wait, WaitReason::Quota);
                    tokio::select! {
                        _ = cancel.cancelled() => return Err(FoundryError::Cancelled),
                        _ = tokio::time::sleep(wait) => {}
                    }
                }
            }
        }
    }

    /// POSTs JSON with the SDK's retry policy, waiting out rate limits; returns the successful
    /// response (body unread).
    pub(crate) async fn post(
        &self,
        api: Api,
        body: &serde_json::Value,
        pace: Pace<'_>,
        cancel: &CancellationToken,
    ) -> Result<reqwest::Response, FoundryError> {
        let url: String = match api {
            Api::Responses => self.responses_url(),
            Api::Chat => self
                .chat_url()
                .map_err(|e| FoundryError::Failed(e.message))?
                .to_string(),
        };
        let bytes = serde_json::to_vec(body).map_err(|e| FoundryError::Failed(e.to_string()))?;
        let headers = self.headers(api);
        let mut attempt = 0u32;
        let mut rate_limited = 0u32;
        let mut rate_limit_waited = Duration::ZERO;
        loop {
            let reservation = self.admit(pace, cancel).await?;
            // A refused request used no quota.
            let release = || {
                if let Some(r) = &reservation {
                    pacing::release(r);
                }
            };
            let send = self
                .http
                .post(&url)
                .headers(headers.clone())
                .body(bytes.clone())
                .send();
            let result = tokio::select! {
                _ = cancel.cancelled() => return Err(FoundryError::Cancelled),
                r = tokio::time::timeout(self.timeout, send) => r,
            };
            let retries_left = self.max_retries.saturating_sub(attempt);
            let (err, delay) = match result {
                Err(_) => (FoundryError::Timeout, None),
                Ok(Err(e)) => {
                    tracing::debug!("Foundry request failed: {e}");
                    (FoundryError::Connection("Connection error.".into()), None)
                }
                Ok(Ok(resp)) if resp.status().is_success() => return Ok(resp),
                Ok(Ok(resp)) if resp.status().as_u16() == 429 => {
                    release();
                    let refused = resp
                        .headers()
                        .get("x-should-retry")
                        .and_then(|v| v.to_str().ok())
                        == Some("false");
                    let response_headers = resp.headers().clone();
                    let body = tokio::select! {
                        _ = cancel.cancelled() => return Err(FoundryError::Cancelled),
                        b = resp.text() => b.unwrap_or_default(),
                    };
                    let message = status_message(429, &body);
                    let wait = pacing::rate_limit_delay(&response_headers, &message)
                        .unwrap_or_else(|| pacing::rate_limit_backoff(rate_limited));
                    let error = FoundryError::Status {
                        status: 429,
                        message,
                    };
                    if refused
                        || pacing::is_quota_exhausted(&error.raw_message())
                        || rate_limit_waited + wait > pacing::RATE_LIMIT_PATIENCE
                    {
                        return Err(error);
                    }
                    tracing::info!(
                        "Foundry rate limit ({}); retrying in {wait:?}",
                        error.raw_message()
                    );
                    pace.report(wait, WaitReason::RateLimited);
                    tokio::select! {
                        _ = cancel.cancelled() => return Err(FoundryError::Cancelled),
                        _ = tokio::time::sleep(wait) => {}
                    }
                    rate_limit_waited += wait;
                    rate_limited += 1;
                    continue;
                }
                Ok(Ok(resp)) => {
                    release();
                    let status = resp.status().as_u16();
                    let retry = should_retry(status, resp.headers());
                    let delay = retry_after(resp.headers());
                    if !(retry && retries_left > 0) {
                        let body = tokio::select! {
                            _ = cancel.cancelled() => return Err(FoundryError::Cancelled),
                            b = resp.text() => b.unwrap_or_default(),
                        };
                        return Err(FoundryError::Status {
                            status,
                            message: status_message(status, &body),
                        });
                    }
                    (
                        FoundryError::Status {
                            status,
                            message: format!("{status}"),
                        },
                        delay,
                    )
                }
            };
            if retries_left == 0 {
                return Err(err);
            }
            let wait = delay.unwrap_or_else(|| default_retry_delay(attempt));
            tracing::info!(
                "Foundry request failed ({}); retrying in {:?}",
                err.raw_message(),
                wait
            );
            tokio::select! {
                _ = cancel.cancelled() => return Err(FoundryError::Cancelled),
                _ = tokio::time::sleep(wait) => {}
            }
            attempt += 1;
        }
    }

    /// A non-streaming chat completion; returns the parsed JSON body.
    pub(crate) async fn chat_json(
        &self,
        body: &serde_json::Value,
        cancel: &CancellationToken,
    ) -> Result<serde_json::Value, FoundryError> {
        // One-off requests: the body's size stands in for its prompt.
        let output = body["max_completion_tokens"].as_u64().unwrap_or(0);
        let cost = (body.to_string().len() as u64 / 4).saturating_add(output);
        let pace = Pace { cost, notify: None };
        let resp = self.post(Api::Chat, body, pace, cancel).await?;
        let text = tokio::select! {
            _ = cancel.cancelled() => return Err(FoundryError::Cancelled),
            t = resp.text() => t.map_err(|_| FoundryError::Connection("Connection error.".into()))?,
        };
        serde_json::from_str(&text)
            .map_err(|e| FoundryError::Failed(format!("Invalid JSON response: {e}")))
    }
}

fn should_retry(status: u16, headers: &HeaderMap) -> bool {
    match headers.get("x-should-retry").and_then(|v| v.to_str().ok()) {
        Some("true") => return true,
        Some("false") => return false,
        _ => {}
    }
    matches!(status, 408 | 409 | 429) || status >= 500
}

/// `retry-after-ms`, then `retry-after` in seconds; ignored unless within 0–60 s.
fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    let get = |k: &str| {
        headers
            .get(k)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<f64>().ok())
    };
    let ms = get("retry-after-ms").or_else(|| get("retry-after").map(|s| s * 1000.0))?;
    (ms.is_finite() && (0.0..=60_000.0).contains(&ms)).then(|| Duration::from_millis(ms as u64))
}

/// 0.5 s, 1 s, 2 s … up to 8 s, minus up to 25 % jitter (as the SDK).
fn default_retry_delay(retry: u32) -> Duration {
    let secs = (0.5 * 2f64.powi(retry as i32)).min(8.0);
    let jitter = 1.0 - (uuid::Uuid::new_v4().as_bytes()[0] as f64 / 255.0) * 0.25;
    Duration::from_secs_f64(secs * jitter)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sdk_style_messages() {
        assert_eq!(
            status_message(
                404,
                r#"{"error":{"code":"404","message":"Resource not found"}}"#
            ),
            "404 Resource not found"
        );
        assert_eq!(status_message(500, ""), "500 status code (no body)");
        assert_eq!(status_message(502, "Bad gateway"), "502 Bad gateway");
        assert_eq!(
            status_message(400, r#"{"detail":"x"}"#),
            "400 status code (no body)"
        );
        assert_eq!(
            status_message(400, r#"{"error":{"code":"x"}}"#),
            r#"400 {"code":"x"}"#
        );
    }

    #[test]
    fn urls() {
        let cfg = FoundryConfig {
            endpoint: "https://res.openai.azure.com/".into(),
            api_key: "k".into(),
            api_version: "2025-04-01-preview".into(),
            deployment: "gpt 5".into(),
            model_name: "m".into(),
            max_completion_tokens: 1,
            context_limit: None,
            tokens_per_minute: None,
            input_price_per_1m: None,
            output_price_per_1m: None,
        };
        let c = FoundryClient::new(cfg).unwrap();
        assert_eq!(
            c.responses_url(),
            "https://res.openai.azure.com/openai/v1/responses"
        );
        assert_eq!(
            c.chat_url().unwrap().as_str(),
            "https://res.openai.azure.com/openai/deployments/gpt%205/chat/completions?api-version=2025-04-01-preview"
        );
    }
}
