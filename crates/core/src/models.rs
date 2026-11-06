//! The model catalog: context limit and prices per model, so they don't have to be typed into
//! Settings or environment variables. Built-in entries are in `crates/core/data/models.json`;
//! `models` in `settings.json` adds entries or replaces built-in ones (same `id`).
//!
//! A deployment is matched by its model name, else its deployment name: equal to an entry's
//! `id` or one of its `aliases` (ignoring case), or the `id` followed by a dated or numbered
//! version (`gpt-6-astra-2026-05-01`).

use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

use crate::logic::types::LongContextRates;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    /// Input tokens the model accepts (the context bar's 100%).
    pub context_limit: u64,
    /// USD per 1M tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_price_per_1m: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_input_price_per_1m: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_price_per_1m: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long_context: Option<LongContextPricing>,
    /// Where the numbers come from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// Rates for a whole request once its input passes `above_input_tokens`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LongContextPricing {
    pub above_input_tokens: u64,
    pub input_price_per_1m: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_input_price_per_1m: Option<f64>,
    pub output_price_per_1m: f64,
}

impl LongContextPricing {
    pub fn rates(&self) -> LongContextRates {
        LongContextRates {
            above: self.above_input_tokens,
            in_price: self.input_price_per_1m,
            cached_price: self.cached_input_price_per_1m,
            out_price: self.output_price_per_1m,
        }
    }
}

static BUILTIN: LazyLock<Vec<ModelInfo>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../data/models.json")).expect("data/models.json is valid")
});

/// The models that ship with the app.
pub fn builtin() -> &'static [ModelInfo] {
    &BUILTIN
}

impl ModelInfo {
    fn matches(&self, name: &str) -> bool {
        let name = name.trim().to_lowercase();
        std::iter::once(&self.id)
            .chain(&self.aliases)
            .map(|n| n.to_lowercase())
            .any(|n| {
                name == n
                    || name
                        .strip_prefix(&n)
                        .and_then(|rest| rest.strip_prefix('-'))
                        .is_some_and(|v| v.starts_with(|c: char| c.is_ascii_digit()))
            })
    }
}

/// The entry for a deployment: `custom` entries first, then the built-in ones; the model name
/// is tried before the deployment name.
pub fn find<'a>(custom: &'a [ModelInfo], names: &[&str]) -> Option<&'a ModelInfo> {
    names
        .iter()
        .filter(|n| !n.trim().is_empty())
        .find_map(|name| {
            custom
                .iter()
                .chain(
                    builtin()
                        .iter()
                        .filter(|b| !custom.iter().any(|c| c.id == b.id)),
                )
                .find(|m| m.matches(name))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_catalog_parses_and_matches() {
        assert!(!builtin().is_empty());
        let astra = find(&[], &["GPT-6-Astra"]).unwrap();
        assert_eq!(astra.id, "gpt-6-astra");
        assert!(find(&[], &["gpt-6-astra-2026-05-01"]).is_some());
        // Another model with the same prefix isn't taken for it.
        assert!(find(&[], &["gpt-6-astra-mini"]).is_none());
        // The deployment name is tried when the model name is unknown.
        assert_eq!(
            find(&[], &["my-model", "gpt-6-astra"]).map(|m| m.id.as_str()),
            Some("gpt-6-astra")
        );
        for m in builtin() {
            assert!(m.context_limit > 0, "{}", m.id);
        }
    }

    #[test]
    fn custom_entries_replace_builtin_ones() {
        let mine = ModelInfo {
            id: "gpt-6-astra".into(),
            label: "Astra (EU)".into(),
            aliases: vec!["astra-eu".into()],
            context_limit: 500_000,
            input_price_per_1m: Some(12.0),
            cached_input_price_per_1m: None,
            output_price_per_1m: Some(60.0),
            long_context: None,
            source: None,
        };
        let custom = [mine];
        assert_eq!(find(&custom, &["gpt-6-astra"]).unwrap().label, "Astra (EU)");
        assert_eq!(find(&custom, &["astra-eu"]).unwrap().context_limit, 500_000);
    }
}
