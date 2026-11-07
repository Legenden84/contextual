//! App settings: `settings.json` in the OS config dir, holding the Foundry configuration
//! *without* the API key. Environment variables (same names as the old `.env.local`) override
//! the file. The key itself lives in a [`SecretStore`] (OS keychain in the app; in-memory and
//! env-var stores here).

use crate::llm::config::{self, ConfigProblems, FoundryConfig, read_foundry_config};
use crate::logic::context::FALLBACK_CONTEXT_LIMIT;
use crate::logic::defaults::{
    BUILTIN_PRESETS, DEFAULT_EXCLUSIONS, LEGACY_EXCLUSIONS, LEGACY_OUTPUT_PROMPTS, OUTPUT_PROMPT,
    default_presets, placeholder_deployment,
};
use crate::logic::types::Deployment;
use crate::models::{self, ModelInfo};
use crate::{Error, Result};
use contextual_protocol::fs::{ExclusionPreset, ScanOptions};
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
    /// Framework exclusions, applied to projects that contain one of a preset's markers.
    pub presets: Vec<ExclusionPreset>,
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
            presets: default_presets(),
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
            presets: self.presets.clone(),
        }
    }

    /// The exclusion defaults (general list and framework presets); the rest is kept.
    pub fn reset_exclusions(&mut self) {
        let d = Self::default();
        self.exclusions = d.exclusions;
        self.presets = d.presets;
    }

    /// Fixes values an older version or a hand edit may have left (like `readSettings` did).
    fn normalise(&mut self) {
        // The old app's unedited list becomes the general list; its framework globs are presets.
        if self
            .exclusions
            .iter()
            .map(String::as_str)
            .eq(LEGACY_EXCLUSIONS.iter().copied())
        {
            self.exclusions = DEFAULT_EXCLUSIONS.iter().map(|s| s.to_string()).collect();
        }
        // Built-in presets added since the file was saved (built-ins can be turned off, not
        // removed).
        for b in BUILTIN_PRESETS {
            if !self.presets.iter().any(|p| p.id == b.id) {
                self.presets.push(b.to_preset());
            }
        }
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

/// How chats are sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ChatSettings {
    /// The model gets the project tree and asks for the files and skills it needs.
    pub model_picks_files: bool,
}

impl Default for ChatSettings {
    fn default() -> Self {
        Self {
            model_picks_files: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePreference {
    #[default]
    Dark,
    Light,
    Moss,
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
    #[serde(default)]
    pub chat: ChatSettings,
    /// Models added to the catalog, or replacing built-in entries with the same id.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<ModelInfo>,
    #[serde(flatten)]
    pub other: serde_json::Map<String, serde_json::Value>,
}

impl Settings {
    /// The model catalog entry for the configured deployment.
    pub fn model(&self, config: &FoundryConfig) -> Option<&ModelInfo> {
        models::find(&self.models, &[&config.model_name, &config.deployment])
    }

    /// The deployment the context bar and cost estimate use: limit and prices from Settings or
    /// environment variables when set, else from the model catalog.
    pub fn deployment(&self, config: Option<&FoundryConfig>) -> Deployment {
        let Some(c) = config else {
            return placeholder_deployment();
        };
        let m = self.model(c);
        // Typed prices replace the catalog's, cached and long-context rates included.
        let own_prices = c.input_price_per_1m.is_some() || c.output_price_per_1m.is_some();
        let catalog = m.filter(|_| !own_prices);
        Deployment {
            name: c.deployment.clone(),
            limit: c
                .context_limit
                .or(m.map(|m| m.context_limit))
                .unwrap_or(FALLBACK_CONTEXT_LIMIT),
            in_price: c
                .input_price_per_1m
                .or(catalog.and_then(|m| m.input_price_per_1m))
                .unwrap_or(0.0),
            out_price: c
                .output_price_per_1m
                .or(catalog.and_then(|m| m.output_price_per_1m))
                .unwrap_or(0.0),
            cached_price: catalog.and_then(|m| m.cached_input_price_per_1m),
            long_context: catalog.and_then(|m| m.long_context.as_ref().map(|l| l.rates())),
            model: m.map(|m| m.label.clone()),
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

    #[test]
    fn theme_preferences_round_trip_and_keep_legacy_values() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path().join("settings.json"));
        assert_eq!(store.load().unwrap().theme, ThemePreference::Dark);
        for (name, theme) in [
            ("dark", ThemePreference::Dark),
            ("light", ThemePreference::Light),
            ("moss", ThemePreference::Moss),
            ("system", ThemePreference::System),
        ] {
            let settings: Settings =
                serde_json::from_value(serde_json::json!({ "theme": name })).unwrap();
            assert_eq!(settings.theme, theme);
            store.save(&settings).unwrap();
            assert_eq!(store.load().unwrap().theme, theme);
            assert_eq!(serde_json::to_value(&settings).unwrap()["theme"], name);
        }
    }

    #[test]
    fn old_exclusions_are_upgraded_and_new_presets_added() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path().join("settings.json"));
        // Saved by an older version: the old single list, no presets.
        let old = serde_json::json!({
            "workspace": { "exclusions": LEGACY_EXCLUSIONS }
        });
        std::fs::write(store.path(), old.to_string()).unwrap();
        let s = store.load().unwrap();
        assert_eq!(s.workspace, WorkspaceSettings::default());

        // Edited lists and presets are kept; missing built-ins are added back.
        let mut edited = WorkspaceSettings {
            exclusions: vec!["docs/**".into()],
            ..WorkspaceSettings::default()
        };
        edited.presets.retain(|p| p.id == "rust");
        edited.presets[0].enabled = false;
        edited.presets[0]
            .exclusions
            .push("**/benches/data/**".into());
        let saved = Settings {
            workspace: edited.clone(),
            ..Settings::default()
        };
        store.save(&saved).unwrap();
        let s = store.load().unwrap().workspace;
        assert_eq!(s.exclusions, edited.exclusions);
        assert_eq!(s.presets[0], edited.presets[0]);
        assert_eq!(s.presets.len(), BUILTIN_PRESETS.len());

        let mut reset = s.clone();
        reset.reset_exclusions();
        assert_eq!(reset.presets, default_presets());
    }

    #[test]
    fn deployment_takes_limit_and_prices_from_the_catalog() {
        let cfg = |model: &str| {
            read_foundry_config(|k| match k {
                config::ENV_ENDPOINT => Some("https://res.openai.azure.com/".into()),
                config::ENV_API_KEY => Some("k".into()),
                config::ENV_API_VERSION => Some("2025-04-01-preview".into()),
                config::ENV_DEPLOYMENT => Some("prod".into()),
                config::ENV_MODEL_NAME => Some(model.into()),
                _ => None,
            })
            .unwrap()
        };
        let s = Settings::default();
        let d = s.deployment(Some(&cfg("gpt-6-astra")));
        assert_eq!(d.model.as_deref(), Some("GPT-6 Astra"));
        assert_eq!((d.limit, d.in_price, d.out_price), (922_000, 10.0, 50.0));
        // 100k in (40k cached), 2k out; past 272k the long-context rates apply to everything.
        assert!((d.cost(100_000, 40_000, 2_000) - (0.6 + 0.04 + 0.1)).abs() < 1e-9);
        assert!(d.rates(300_000).long_context);
        assert!((d.cost(300_000, 0, 0) - 6.0).abs() < 1e-9);

        // Typed values win; typed prices drop the catalog's cached and long-context rates.
        let mut own = cfg("gpt-6-astra");
        own.context_limit = Some(200_000);
        own.input_price_per_1m = Some(5.0);
        let d = s.deployment(Some(&own));
        assert_eq!((d.limit, d.in_price, d.out_price), (200_000, 5.0, 0.0));
        assert_eq!((d.cached_price, d.long_context), (None, None));

        // Unknown model: the fallback limit, no prices.
        let d = s.deployment(Some(&cfg("something-else")));
        assert_eq!(
            (d.limit, d.model.as_deref()),
            (FALLBACK_CONTEXT_LIMIT, None)
        );
        assert!(!d.priced());
    }

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
