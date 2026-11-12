//! The model catalog: context limit, prices, tier and description per model, for models the
//! user describes once under `models` in `settings.json` instead of per deployment. The app
//! ships no entries of its own: every model is defined by the user.
//!
//! A deployment is matched by its model name, else its deployment name: equal to an entry's
//! `id` or one of its `aliases` (ignoring case), or the `id` followed by a dated or numbered
//! version (`gpt-6-astra-2026-05-01`).

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
    /// How capable the model is next to the others the app routes between: higher is more
    /// capable (and usually more expensive).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<u8>,
    /// What the model is good for; the routing model chooses from this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
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

/// The entry for a deployment among the user's `catalog` entries; the model name is tried
/// before the deployment name.
pub fn find<'a>(catalog: &'a [ModelInfo], names: &[&str]) -> Option<&'a ModelInfo> {
    names
        .iter()
        .filter(|n| !n.trim().is_empty())
        .find_map(|name| catalog.iter().find(|m| m.matches(name)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str) -> ModelInfo {
        ModelInfo {
            id: id.into(),
            label: id.to_uppercase(),
            aliases: Vec::new(),
            context_limit: 500_000,
            input_price_per_1m: Some(12.0),
            cached_input_price_per_1m: None,
            output_price_per_1m: Some(60.0),
            long_context: None,
            tier: Some(4),
            description: None,
            source: None,
        }
    }

    #[test]
    fn nothing_is_known_until_the_user_defines_it() {
        assert_eq!(find(&[], &["gpt-6-astra"]), None);
    }

    #[test]
    fn entries_match_by_id_alias_and_version() {
        let mut astra = entry("gpt-6-astra");
        astra.aliases = vec!["astra-eu".into()];
        let catalog = [astra, entry("gpt-6.1-sol")];
        assert_eq!(find(&catalog, &["GPT-6-Astra"]).unwrap().id, "gpt-6-astra");
        assert!(find(&catalog, &["gpt-6-astra-2026-05-01"]).is_some());
        assert_eq!(
            find(&catalog, &["astra-eu"]).unwrap().context_limit,
            500_000
        );
        // Another model with the same prefix isn't taken for it.
        assert!(find(&catalog, &["gpt-6-astra-mini"]).is_none());
        // The deployment name is tried when the model name is unknown.
        assert_eq!(
            find(&catalog, &["my-model", "gpt-6-astra"]).map(|m| m.id.as_str()),
            Some("gpt-6-astra")
        );
        assert_eq!(
            find(&catalog, &["", "gpt-6.1-sol-2026-09-01"]).unwrap().id,
            "gpt-6.1-sol"
        );
    }
}
