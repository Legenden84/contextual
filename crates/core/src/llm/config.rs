//! Azure AI Foundry settings and their validation (port of `config.ts`).
//! Variable names follow the `openai` SDK's AzureOpenAI defaults where one exists.

use crate::error::{Error, ErrorCode};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FoundryConfig {
    /// Resource endpoint, e.g. `https://myresource.openai.azure.com/`.
    pub endpoint: String,
    /// Never serialised (health output, settings file).
    #[serde(skip)]
    pub api_key: String,
    pub api_version: String,
    /// Deployment name, sent as `model` on every request.
    pub deployment: String,
    /// Display name of the underlying model.
    pub model_name: String,
    pub max_completion_tokens: u32,
    /// Context window used by the context bar. Unset until confirmed for the model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_limit: Option<u64>,
    /// USD per 1M input / output tokens, for cost estimates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_price_per_1m: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_price_per_1m: Option<f64>,
}

pub const ENV_ENDPOINT: &str = "AZURE_OPENAI_ENDPOINT";
pub const ENV_API_KEY: &str = "AZURE_OPENAI_API_KEY";
pub const ENV_API_VERSION: &str = "OPENAI_API_VERSION";
pub const ENV_DEPLOYMENT: &str = "AZURE_OPENAI_DEPLOYMENT";
pub const ENV_MODEL_NAME: &str = "AZURE_OPENAI_MODEL_NAME";
pub const ENV_MAX_COMPLETION_TOKENS: &str = "AZURE_OPENAI_MAX_COMPLETION_TOKENS";
pub const ENV_CONTEXT_LIMIT: &str = "AZURE_OPENAI_CONTEXT_LIMIT";
pub const ENV_INPUT_PRICE: &str = "AZURE_OPENAI_INPUT_PRICE_PER_1M";
pub const ENV_OUTPUT_PRICE: &str = "AZURE_OPENAI_OUTPUT_PRICE_PER_1M";

pub const REQUIRED_ENV: [&str; 4] = [ENV_ENDPOINT, ENV_API_KEY, ENV_API_VERSION, ENV_DEPLOYMENT];
/// Every variable the loader reads.
pub const ALL_ENV: [&str; 9] = [
    ENV_ENDPOINT,
    ENV_API_KEY,
    ENV_API_VERSION,
    ENV_DEPLOYMENT,
    ENV_MODEL_NAME,
    ENV_MAX_COMPLETION_TOKENS,
    ENV_CONTEXT_LIMIT,
    ENV_INPUT_PRICE,
    ENV_OUTPUT_PRICE,
];

pub const DEFAULT_MAX_COMPLETION_TOKENS: u32 = 16384;

/// Why the configuration can't be used: variables that are missing or invalid.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigProblems {
    pub missing: Vec<String>,
    pub invalid: Vec<String>,
}

impl ConfigProblems {
    /// "Foundry is not configured: missing X; invalid Y. See .env.example." (as the old server).
    pub fn message(&self) -> String {
        let mut parts = Vec::new();
        if !self.missing.is_empty() {
            parts.push(format!("missing {}", self.missing.join(", ")));
        }
        if !self.invalid.is_empty() {
            parts.push(format!("invalid {}", self.invalid.join(", ")));
        }
        format!(
            "Foundry is not configured: {}. See .env.example.",
            parts.join("; ")
        )
    }

    pub fn into_error(self) -> Error {
        Error::new(ErrorCode::NotConfigured, self.message())
    }
}

/// HTTPS only, except a local mock or proxy on this machine (`http://localhost`, `127.0.0.1`, `[::1]`).
pub fn is_allowed_endpoint(endpoint: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(endpoint) else {
        return false;
    };
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    url.scheme() == "https" || (local && url.scheme() == "http")
}

/// A positive number (`Number(raw) > 0`); `None` when unset, `Err` when invalid.
fn positive(raw: Option<String>) -> Result<Option<f64>, ()> {
    let Some(raw) = raw.map(|r| r.trim().to_string()).filter(|r| !r.is_empty()) else {
        return Ok(None);
    };
    match raw.parse::<f64>() {
        Ok(n) if n.is_finite() && n > 0.0 => Ok(Some(n)),
        _ => Err(()),
    }
}

/// Positive whole number (token counts).
fn positive_int(raw: Option<String>) -> Result<Option<u64>, ()> {
    match positive(raw)? {
        None => Ok(None),
        Some(n) if n.fract() == 0.0 && n <= u32::MAX as f64 * 1024.0 => Ok(Some(n as u64)),
        Some(_) => Err(()),
    }
}

/// Reads and validates the configuration from variables named like the old `.env.local`.
/// `get` returns a variable's value (e.g. `|k| std::env::var(k).ok()`).
pub fn read_foundry_config(
    get: impl Fn(&str) -> Option<String>,
) -> Result<FoundryConfig, ConfigProblems> {
    let value = |k: &str| {
        get(k)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let mut p = ConfigProblems {
        missing: REQUIRED_ENV
            .iter()
            .filter(|k| value(k).is_none())
            .map(|k| k.to_string())
            .collect(),
        invalid: Vec::new(),
    };
    let endpoint = value(ENV_ENDPOINT);
    if let Some(e) = &endpoint {
        if !is_allowed_endpoint(e) {
            p.invalid.push(ENV_ENDPOINT.into());
        }
    }
    let mut check = |key: &str, r: Result<Option<u64>, ()>| match r {
        Ok(v) => v,
        Err(()) => {
            p.invalid.push(key.into());
            None
        }
    };
    let max_tokens = check(
        ENV_MAX_COMPLETION_TOKENS,
        positive_int(get(ENV_MAX_COMPLETION_TOKENS)).and_then(|v| match v {
            Some(n) if n > u32::MAX as u64 => Err(()),
            other => Ok(other),
        }),
    );
    let context_limit = check(ENV_CONTEXT_LIMIT, positive_int(get(ENV_CONTEXT_LIMIT)));
    let mut price = |key: &str| match positive(get(key)) {
        Ok(v) => v,
        Err(()) => {
            p.invalid.push(key.into());
            None
        }
    };
    let input_price_per_1m = price(ENV_INPUT_PRICE);
    let output_price_per_1m = price(ENV_OUTPUT_PRICE);

    if !p.missing.is_empty() || !p.invalid.is_empty() {
        return Err(p);
    }
    let deployment = value(ENV_DEPLOYMENT).unwrap_or_default();
    Ok(FoundryConfig {
        endpoint: endpoint.unwrap_or_default(),
        api_key: value(ENV_API_KEY).unwrap_or_default(),
        api_version: value(ENV_API_VERSION).unwrap_or_default(),
        model_name: value(ENV_MODEL_NAME).unwrap_or_else(|| deployment.clone()),
        deployment,
        max_completion_tokens: max_tokens
            .map(|n| n as u32)
            .unwrap_or(DEFAULT_MAX_COMPLETION_TOKENS),
        context_limit,
        input_price_per_1m,
        output_price_per_1m,
    })
}

/// [`read_foundry_config`] over the process environment.
pub fn foundry_config_from_env() -> Result<FoundryConfig, ConfigProblems> {
    read_foundry_config(|k| std::env::var(k).ok())
}

impl FoundryConfig {
    /// The variables this config corresponds to (for re-validation or merging with overrides).
    pub fn to_vars(&self) -> Vec<(&'static str, String)> {
        let mut v = vec![
            (ENV_ENDPOINT, self.endpoint.clone()),
            (ENV_API_KEY, self.api_key.clone()),
            (ENV_API_VERSION, self.api_version.clone()),
            (ENV_DEPLOYMENT, self.deployment.clone()),
            (ENV_MODEL_NAME, self.model_name.clone()),
            (
                ENV_MAX_COMPLETION_TOKENS,
                self.max_completion_tokens.to_string(),
            ),
        ];
        if let Some(n) = self.context_limit {
            v.push((ENV_CONTEXT_LIMIT, n.to_string()));
        }
        if let Some(n) = self.input_price_per_1m {
            v.push((ENV_INPUT_PRICE, n.to_string()));
        }
        if let Some(n) = self.output_price_per_1m {
            v.push((ENV_OUTPUT_PRICE, n.to_string()));
        }
        v
    }

    /// Validates a config built in code (e.g. from settings) with the same rules as the loader.
    pub fn validate(&self) -> Result<(), ConfigProblems> {
        let vars = self.to_vars();
        read_foundry_config(|k| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| v.clone()))
            .map(|_| ())
    }
}

/// The `/api/health` shape: `{ configured: true, endpoint, apiVersion, … }` without the key, or
/// `{ configured: false, missing, invalid }`.
pub fn public_config(result: &Result<FoundryConfig, ConfigProblems>) -> serde_json::Value {
    match result {
        Ok(cfg) => {
            let mut v = serde_json::to_value(cfg).unwrap_or_default();
            if let Some(o) = v.as_object_mut() {
                o.insert("configured".into(), true.into());
            }
            v
        }
        Err(p) => {
            serde_json::json!({ "configured": false, "missing": p.missing, "invalid": p.invalid })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| m.get(k).cloned()
    }

    const BASE: [(&str, &str); 4] = [
        (ENV_ENDPOINT, "https://x.openai.azure.com/"),
        (ENV_API_KEY, "secret"),
        (ENV_API_VERSION, "2025-04-01-preview"),
        (ENV_DEPLOYMENT, "gpt-5"),
    ];

    #[test]
    fn valid_config_with_defaults() {
        let cfg = read_foundry_config(env(&BASE)).unwrap();
        assert_eq!(cfg.model_name, "gpt-5");
        assert_eq!(cfg.max_completion_tokens, 16384);
        assert_eq!(cfg.context_limit, None);
        assert!(cfg.validate().is_ok());
        let health = public_config(&Ok(cfg));
        assert_eq!(health["configured"], true);
        assert!(health.get("apiKey").is_none());
        assert_eq!(health["maxCompletionTokens"], 16384);
    }

    #[test]
    fn missing_and_invalid() {
        let p = read_foundry_config(env(&[
            (ENV_ENDPOINT, "http://example.com"),
            (ENV_DEPLOYMENT, "  "),
            (ENV_CONTEXT_LIMIT, "-5"),
            (ENV_INPUT_PRICE, "abc"),
            (ENV_OUTPUT_PRICE, "2.5"),
        ]))
        .unwrap_err();
        assert_eq!(
            p.missing,
            vec![ENV_API_KEY, ENV_API_VERSION, ENV_DEPLOYMENT]
        );
        assert_eq!(
            p.invalid,
            vec![ENV_ENDPOINT, ENV_CONTEXT_LIMIT, ENV_INPUT_PRICE]
        );
        assert_eq!(
            p.message(),
            "Foundry is not configured: missing AZURE_OPENAI_API_KEY, OPENAI_API_VERSION, AZURE_OPENAI_DEPLOYMENT; invalid AZURE_OPENAI_ENDPOINT, AZURE_OPENAI_CONTEXT_LIMIT, AZURE_OPENAI_INPUT_PRICE_PER_1M. See .env.example."
        );
    }

    #[test]
    fn endpoint_rules() {
        assert!(is_allowed_endpoint("https://a.b/"));
        assert!(is_allowed_endpoint("http://localhost:8080"));
        assert!(is_allowed_endpoint("http://127.0.0.1:1/"));
        assert!(is_allowed_endpoint("http://[::1]:9/"));
        assert!(!is_allowed_endpoint("http://localhost.evil.com"));
        assert!(!is_allowed_endpoint("ftp://localhost"));
        assert!(!is_allowed_endpoint("not a url"));
    }
}
