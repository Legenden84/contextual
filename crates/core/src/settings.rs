//! App settings: `settings.json` in the OS config dir, holding the Foundry configuration
//! *without* the API key. Environment variables (same names as the old `.env.local`) override
//! the file. The key itself lives in a [`SecretStore`] (OS keychain in the app; in-memory and
//! env-var stores here).

use crate::llm::config::{self, ConfigProblems, FoundryConfig, read_foundry_config};
use crate::logic::context::FALLBACK_CONTEXT_LIMIT;
use crate::logic::defaults::{
    DEFAULT_EXCLUSIONS, LEGACY_OUTPUT_PROMPTS, OUTPUT_PROMPT, placeholder_deployment,
};
use crate::logic::types::Deployment;
use crate::{Error, Result};
use contextual_protocol::fs::ScanOptions;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Secret name of the Foundry API key.
pub const API_KEY_SECRET: &str = "azure-openai-api-key";

/// Foundry settings as stored (all optional; validation happens when resolving).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FoundrySettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deployment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_completion_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_limit: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_price_per_1m: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_price_per_1m: Option<f64>,
}

impl FoundrySettings {
    /// The non-secret parts of a config (e.g. after importing a `.env.local`).
    pub fn from_config(cfg: &FoundryConfig) -> Self {
        Self {
            endpoint: Some(cfg.endpoint.clone()),
            api_version: Some(cfg.api_version.clone()),
            deployment: Some(cfg.deployment.clone()),
            model_name: Some(cfg.model_name.clone()),
            max_completion_tokens: Some(cfg.max_completion_tokens),
            context_limit: cfg.context_limit,
            input_price_per_1m: cfg.input_price_per_1m,
            output_price_per_1m: cfg.output_price_per_1m,
        }
    }

    fn var(&self, name: &str) -> Option<String> {
        match name {
            config::ENV_ENDPOINT => self.endpoint.clone(),
            config::ENV_API_VERSION => self.api_version.clone(),
            config::ENV_DEPLOYMENT => self.deployment.clone(),
            config::ENV_MODEL_NAME => self.model_name.clone(),
            config::ENV_MAX_COMPLETION_TOKENS => self.max_completion_tokens.map(|n| n.to_string()),
            config::ENV_CONTEXT_LIMIT => self.context_limit.map(|n| n.to_string()),
            config::ENV_INPUT_PRICE => self.input_price_per_1m.map(|n| n.to_string()),
            config::ENV_OUTPUT_PRICE => self.output_price_per_1m.map(|n| n.to_string()),
            _ => None,
        }
    }
}

/// What gets scanned and how the model is asked to answer (Settings → Exclusions, Output format).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkspaceSettings {
    pub exclusions: Vec<String>,
    pub max_file_size_kb: f64,
    pub respect_gitignore: bool,
    pub skip_binary: bool,
    pub include_hidden: bool,
    /// The user's output-format prompt (the built-in edit format is appended separately).
    pub output_prompt: String,
}

impl Default for WorkspaceSettings {
    fn default() -> Self {
        Self {
            exclusions: DEFAULT_EXCLUSIONS.iter().map(|s| s.to_string()).collect(),
            max_file_size_kb: 256.0,
            respect_gitignore: true,
            skip_binary: true,
            include_hidden: false,
            output_prompt: OUTPUT_PROMPT.to_string(),
        }
    }
}

impl WorkspaceSettings {
    pub fn scan_options(&self) -> ScanOptions {
        ScanOptions {
            exclusions: self.exclusions.clone(),
            max_file_size_kb: self.max_file_size_kb,
            respect_gitignore: self.respect_gitignore,
            skip_binary: self.skip_binary,
            include_hidden: self.include_hidden,
        }
    }

    /// Fixes values an older version or a hand edit may have left (like `readSettings` did).
    fn normalise(&mut self) {
        // Also catches NaN.
        if self.max_file_size_kb.is_nan() || self.max_file_size_kb <= 0.0 {
            self.max_file_size_kb = 256.0;
        }
        // An unedited old default prompt is replaced by the current one; the user's own is kept.
        if self.output_prompt.trim().is_empty()
            || LEGACY_OUTPUT_PROMPTS.contains(&self.output_prompt.as_str())
        {
            self.output_prompt = OUTPUT_PROMPT.to_string();
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePreference {
    #[default]
    Dark,
    Light,
    /// Follow the OS appearance.
    System,
}

/// Contents of `settings.json`. Unknown keys are kept, so an older app doesn't drop newer settings.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default)]
    pub foundry: FoundrySettings,
    #[serde(default)]
    pub workspace: WorkspaceSettings,
    #[serde(default)]
    pub theme: ThemePreference,
    #[serde(flatten)]
    pub other: serde_json::Map<String, serde_json::Value>,
}

impl Settings {
    /// The deployment the context bar and cost estimate use, from the resolved Foundry config.
    pub fn deployment(config: Option<&FoundryConfig>) -> Deployment {
        match config {
            Some(c) => Deployment {
                name: c.deployment.clone(),
                limit: c.context_limit.unwrap_or(FALLBACK_CONTEXT_LIMIT),
                in_price: c.input_price_per_1m.unwrap_or(0.0),
                out_price: c.output_price_per_1m.unwrap_or(0.0),
            },
            None => placeholder_deployment(),
        }
    }
}

/// Reads and writes `settings.json`.
#[derive(Debug, Clone)]
pub struct SettingsStore {
    path: PathBuf,
}

/// `<config dir>/settings.json` for this OS.
pub fn default_settings_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "contextual")
        .map(|d| d.config_dir().join("settings.json"))
}

impl SettingsStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn open_default() -> Result<Self> {
        default_settings_path()
            .map(Self::new)
            .ok_or_else(|| Error::internal("No home directory."))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The settings, or defaults when the file doesn't exist yet.
    pub fn load(&self) -> Result<Settings> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => {
                let mut s: Settings = serde_json::from_str(&text).map_err(|e| {
                    Error::invalid(format!("{} is not valid: {e}", self.path.display()))
                })?;
                s.workspace.normalise();
                Ok(s)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
            Err(e) => Err(e.into()),
        }
    }

    /// Writes atomically (temp file + rename).
    pub fn save(&self, settings: &Settings) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self
            .path
            .with_extension(format!("json.{}.tmp", std::process::id()));
        std::fs::write(&tmp, serde_json::to_vec_pretty(settings)?)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

/// Where secrets (the API key) are kept.
pub trait SecretStore: Send + Sync {
    fn get(&self, name: &str) -> Result<Option<String>>;
    fn set(&self, name: &str, value: &str) -> Result<()>;
    fn delete(&self, name: &str) -> Result<()>;
}

/// Secrets in memory (tests, or when no keychain is available).
#[derive(Debug, Default)]
pub struct MemorySecretStore {
    values: Mutex<HashMap<String, String>>,
}

impl MemorySecretStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretStore for MemorySecretStore {
    fn get(&self, name: &str) -> Result<Option<String>> {
        Ok(self
            .values
            .lock()
            .map_err(|_| Error::internal("secret store poisoned"))?
            .get(name)
            .cloned())
    }
    fn set(&self, name: &str, value: &str) -> Result<()> {
        self.values
            .lock()
            .map_err(|_| Error::internal("secret store poisoned"))?
            .insert(name.into(), value.into());
        Ok(())
    }
    fn delete(&self, name: &str) -> Result<()> {
        self.values
            .lock()
            .map_err(|_| Error::internal("secret store poisoned"))?
            .remove(name);
        Ok(())
    }
}

/// Read-only secrets from environment variables; [`API_KEY_SECRET`] maps to `AZURE_OPENAI_API_KEY`.
#[derive(Debug, Default)]
pub struct EnvSecretStore;

impl EnvSecretStore {
    fn var_name(name: &str) -> String {
        if name == API_KEY_SECRET {
            config::ENV_API_KEY.to_string()
        } else {
            name.to_ascii_uppercase().replace('-', "_")
        }
    }
}

impl SecretStore for EnvSecretStore {
    fn get(&self, name: &str) -> Result<Option<String>> {
        Ok(std::env::var(Self::var_name(name))
            .ok()
            .filter(|v| !v.trim().is_empty()))
    }
    fn set(&self, _: &str, _: &str) -> Result<()> {
        Err(Error::invalid(
            "Secrets from environment variables are read-only.",
        ))
    }
    fn delete(&self, _: &str) -> Result<()> {
        Err(Error::invalid(
            "Secrets from environment variables are read-only.",
        ))
    }
}

/// The effective Foundry config: settings file values, overridden by environment variables
/// (`env`, e.g. `|k| std::env::var(k).ok()`); the key from `AZURE_OPENAI_API_KEY` if set, else
/// the secret store. Validated like the old `.env.local`.
pub fn resolve_foundry_config(
    settings: &Settings,
    secrets: &dyn SecretStore,
    env: impl Fn(&str) -> Option<String>,
) -> std::result::Result<FoundryConfig, ConfigProblems> {
    let set = |v: Option<String>| v.filter(|s| !s.trim().is_empty());
    let key = set(env(config::ENV_API_KEY)).or_else(|| secrets.get(API_KEY_SECRET).ok().flatten());
    read_foundry_config(|name| {
        if name == config::ENV_API_KEY {
            return key.clone();
        }
        set(env(name)).or_else(|| settings.foundry.var(name))
    })
}

/// [`resolve_foundry_config`] with the process environment.
pub fn resolve_foundry_config_from_env(
    settings: &Settings,
    secrets: &dyn SecretStore,
) -> std::result::Result<FoundryConfig, ConfigProblems> {
    resolve_foundry_config(settings, secrets, |k| std::env::var(k).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        Settings {
            foundry: FoundrySettings {
                endpoint: Some("https://res.openai.azure.com/".into()),
                api_version: Some("2025-04-01-preview".into()),
                deployment: Some("gpt-5".into()),
                context_limit: Some(400_000),
                ..FoundrySettings::default()
            },
            ..Settings::default()
        }
    }

    #[test]
    fn env_overrides_settings_and_key_from_store() {
        let secrets = MemorySecretStore::new();
        let none = |_: &str| None;
        let p = resolve_foundry_config(&settings(), &secrets, none).unwrap_err();
        assert_eq!(p.missing, vec![config::ENV_API_KEY]);

        secrets.set(API_KEY_SECRET, "k1").unwrap();
        let cfg = resolve_foundry_config(&settings(), &secrets, none).unwrap();
        assert_eq!(cfg.api_key, "k1");
        assert_eq!(cfg.context_limit, Some(400_000));

        let env = |k: &str| match k {
            config::ENV_DEPLOYMENT => Some("other".to_string()),
            config::ENV_API_KEY => Some("k2".to_string()),
            _ => None,
        };
        let cfg = resolve_foundry_config(&settings(), &secrets, env).unwrap();
        assert_eq!(
            (cfg.deployment.as_str(), cfg.api_key.as_str()),
            ("other", "k2")
        );
    }

    #[test]
    fn file_round_trip_keeps_unknown_keys_and_never_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path().join("cfg").join("settings.json"));
        assert_eq!(store.load().unwrap(), Settings::default());
        let mut s = settings();
        s.other.insert("futureSetting".into(), "kept".into());
        store.save(&s).unwrap();
        let text = std::fs::read_to_string(store.path()).unwrap();
        assert!(!text.to_lowercase().contains("apikey"));
        assert_eq!(store.load().unwrap(), s);
    }
}
