//! Bringing data over from the old Node/React app (plan §9.1): the Foundry configuration from a
//! `.env.local` (the key goes to the keychain, the rest to `settings.json`). Saved chats move
//! with `ThreadStore::import_legacy`.

use std::collections::BTreeMap;

use crate::llm::config as cfg;
use crate::settings::{FoundrySettings, ModelSettings};

/// Parses a dotenv file: `KEY=value`, optional `export`, single or double quotes, `#` comments
/// (inline after unquoted values). Later keys win.
pub fn parse_env(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        let value = value.trim();
        let value = if let Some(q) = value.chars().next().filter(|c| *c == '"' || *c == '\'') {
            let inner = &value[1..];
            match inner.find(q) {
                Some(end) => inner[..end].to_string(),
                None => inner.to_string(),
            }
        } else {
            value.split(" #").next().unwrap_or("").trim().to_string()
        };
        out.insert(key.to_string(), value);
    }
    out
}

/// What a `.env.local` contributes.
#[derive(Debug, Clone, PartialEq)]
pub struct EnvImport {
    pub foundry: FoundrySettings,
    pub api_key: Option<String>,
    /// Variables that were set but couldn't be read (e.g. a non-numeric limit).
    pub skipped: Vec<String>,
}

pub fn import_env(text: &str) -> EnvImport {
    let vars = parse_env(text);
    let get = |k: &str| {
        vars.get(k)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let mut skipped = Vec::new();
    let mut num = |k: &str| -> Option<String> {
        let v = get(k)?;
        if v.parse::<f64>().is_ok() {
            Some(v)
        } else {
            skipped.push(k.to_string());
            None
        }
    };
    let foundry = FoundrySettings {
        endpoint: get(cfg::ENV_ENDPOINT),
        api_version: get(cfg::ENV_API_VERSION),
        deployment: get(cfg::ENV_DEPLOYMENT),
        model_name: get(cfg::ENV_MODEL_NAME),
        max_completion_tokens: num(cfg::ENV_MAX_COMPLETION_TOKENS).and_then(|v| v.parse().ok()),
        context_limit: num(cfg::ENV_CONTEXT_LIMIT).and_then(|v| v.parse().ok()),
        input_price_per_1m: num(cfg::ENV_INPUT_PRICE).and_then(|v| v.parse().ok()),
        output_price_per_1m: num(cfg::ENV_OUTPUT_PRICE).and_then(|v| v.parse().ok()),
        tokens_per_minute: num(cfg::ENV_TOKENS_PER_MINUTE).and_then(|v| v.parse().ok()),
        // Every `AZURE_OPENAI_<NAME>_DEPLOYMENT` is a model of its own.
        models: cfg::model_names(vars.keys())
            .into_iter()
            .map(|name| {
                let key = |base: &str| cfg::model_env(&name, base);
                ModelSettings {
                    deployment: get(&key(cfg::ENV_DEPLOYMENT)),
                    model_name: get(&key(cfg::ENV_MODEL_NAME)),
                    tier: num(&key(cfg::ENV_TIER)).and_then(|v| v.parse().ok()),
                    description: get(&key(cfg::ENV_DESCRIPTION)),
                    max_completion_tokens: num(&key(cfg::ENV_MAX_COMPLETION_TOKENS))
                        .and_then(|v| v.parse().ok()),
                    context_limit: num(&key(cfg::ENV_CONTEXT_LIMIT)).and_then(|v| v.parse().ok()),
                    input_price_per_1m: num(&key(cfg::ENV_INPUT_PRICE))
                        .and_then(|v| v.parse().ok()),
                    cached_input_price_per_1m: num(&key(cfg::ENV_CACHED_INPUT_PRICE))
                        .and_then(|v| v.parse().ok()),
                    output_price_per_1m: num(&key(cfg::ENV_OUTPUT_PRICE))
                        .and_then(|v| v.parse().ok()),
                    tokens_per_minute: num(&key(cfg::ENV_TOKENS_PER_MINUTE))
                        .and_then(|v| v.parse().ok()),
                    name,
                }
            })
            .collect(),
        router: get(cfg::ENV_ROUTER).map(|r| r.to_lowercase()),
    };
    EnvImport {
        foundry,
        api_key: get(cfg::ENV_API_KEY),
        skipped,
    }
}

/// Merges imported values over existing settings (only what the file sets).
pub fn merge(into: &mut FoundrySettings, from: &FoundrySettings) {
    macro_rules! take {
        ($($f:ident),*) => { $( if from.$f.is_some() { into.$f = from.$f.clone(); } )* };
    }
    take!(
        endpoint,
        api_version,
        deployment,
        model_name,
        max_completion_tokens,
        context_limit,
        input_price_per_1m,
        output_price_per_1m,
        tokens_per_minute,
        router
    );
    // Models are matched by name; a model the file doesn't mention is kept as it is.
    for model in &from.models {
        let Some(existing) = into
            .models
            .iter_mut()
            .find(|m| m.name.trim().eq_ignore_ascii_case(&model.name))
        else {
            into.models.push(model.clone());
            continue;
        };
        macro_rules! take_model {
            ($($f:ident),*) => { $( if model.$f.is_some() { existing.$f = model.$f.clone(); } )* };
        }
        take_model!(
            deployment,
            model_name,
            tier,
            description,
            max_completion_tokens,
            context_limit,
            input_price_per_1m,
            cached_input_price_per_1m,
            output_price_per_1m,
            tokens_per_minute
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dotenv() {
        let text = "# Foundry\nAZURE_OPENAI_ENDPOINT=https://x.openai.azure.com/\nexport AZURE_OPENAI_API_KEY=\"se#cret\"\n\
                    AZURE_OPENAI_DEPLOYMENT='gpt-5' \nAZURE_OPENAI_CONTEXT_LIMIT=400000 # tokens\nAZURE_OPENAI_INPUT_PRICE_PER_1M=lots\n\
                    BAD KEY=1\nOPENAI_API_VERSION=2025-04-01-preview\n";
        let vars = parse_env(text);
        assert_eq!(vars["AZURE_OPENAI_API_KEY"], "se#cret");
        assert_eq!(vars["AZURE_OPENAI_DEPLOYMENT"], "gpt-5");
        assert_eq!(vars["AZURE_OPENAI_CONTEXT_LIMIT"], "400000");
        assert!(!vars.contains_key("BAD KEY"));

        let imp = import_env(text);
        assert_eq!(imp.api_key.as_deref(), Some("se#cret"));
        assert_eq!(
            imp.foundry.endpoint.as_deref(),
            Some("https://x.openai.azure.com/")
        );
        assert_eq!(imp.foundry.context_limit, Some(400_000));
        assert_eq!(imp.foundry.input_price_per_1m, None);
        assert_eq!(
            imp.skipped,
            vec!["AZURE_OPENAI_INPUT_PRICE_PER_1M".to_string()]
        );

        let mut existing = FoundrySettings {
            model_name: Some("kept".into()),
            deployment: Some("old".into()),
            ..Default::default()
        };
        merge(&mut existing, &imp.foundry);
        assert_eq!(existing.model_name.as_deref(), Some("kept"));
        assert_eq!(existing.deployment.as_deref(), Some("gpt-5"));
    }

    #[test]
    fn imports_one_block_per_model() {
        let text = "AZURE_OPENAI_ENDPOINT=https://x.openai.azure.com/\nAZURE_OPENAI_API_KEY=k\n\
                    OPENAI_API_VERSION=2025-04-01-preview\nAZURE_OPENAI_ROUTER=Terra\n\
                    AZURE_OPENAI_TERRA_DEPLOYMENT=gpt-6.1-terra-code\n\
                    AZURE_OPENAI_TERRA_MODEL_NAME=gpt-6.1-terra\n\
                    AZURE_OPENAI_ASTRA_DEPLOYMENT=gpt-6-astra-code\nAZURE_OPENAI_ASTRA_TIER=4\n\
                    AZURE_OPENAI_ASTRA_DESCRIPTION=\"Hard problems\"\n\
                    AZURE_OPENAI_ASTRA_INPUT_PRICE_PER_1M=twelve\n\
                    AZURE_OPENAI_ASTRA_CACHED_INPUT_PRICE_PER_1M=1\n";
        let imp = import_env(text);
        // No unnamed deployment in the file.
        assert_eq!(imp.foundry.deployment, None);
        assert_eq!(imp.foundry.router.as_deref(), Some("terra"));
        let names: Vec<&str> = imp.foundry.models.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["astra", "terra"]);
        let astra = &imp.foundry.models[0];
        assert_eq!(astra.deployment.as_deref(), Some("gpt-6-astra-code"));
        assert_eq!(astra.tier, Some(4));
        assert_eq!(astra.description.as_deref(), Some("Hard problems"));
        assert_eq!(astra.input_price_per_1m, None);
        assert_eq!(astra.cached_input_price_per_1m, Some(1.0));
        assert_eq!(
            imp.skipped,
            vec!["AZURE_OPENAI_ASTRA_INPUT_PRICE_PER_1M".to_string()]
        );

        // Merging updates a model by name and keeps the ones the file doesn't mention.
        let mut existing = FoundrySettings {
            models: vec![
                ModelSettings {
                    name: "Astra".into(),
                    deployment: Some("old".into()),
                    context_limit: Some(500_000),
                    ..ModelSettings::default()
                },
                ModelSettings {
                    name: "luna".into(),
                    ..ModelSettings::default()
                },
            ],
            ..Default::default()
        };
        merge(&mut existing, &imp.foundry);
        let names: Vec<&str> = existing.models.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["Astra", "luna", "terra"]);
        assert_eq!(
            existing.models[0].deployment.as_deref(),
            Some("gpt-6-astra-code")
        );
        assert_eq!(existing.models[0].context_limit, Some(500_000));
        assert_eq!(existing.router.as_deref(), Some("terra"));
    }
}
