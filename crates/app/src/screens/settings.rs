//! Settings (design spec: ScreenSettings): persistent group navigation beside the Foundry
//! connection, exclusions, output-format prompt and appearance form.

use contextual_core::llm::config::ConfigProblems;
use contextual_core::logic::foundry_status::FoundryStatus;
use contextual_core::settings::{FoundrySettings, ModelSettings, Settings, ThemePreference};
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext, Context, Entity, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window,
    div,
};

use crate::state::{AppModel, ConnectionTest};
use crate::theme::{ActiveTheme, TypeExt, control_height, hsla, px, sp};
use crate::ui::{Button, ButtonSize, ButtonVariant, IconButton, LinkButton, Switch, Tag, TagTone};
use crate::views::env_file_dialog::{EnvFileDialog, EnvFileEvent};
use crate::views::presets_editor::PresetsEditor;

const LABEL_W: f32 = 200.0;
const FIELD_W: f32 = 420.0;
const SETTINGS_MAX_W: f32 = contextual_theme::layout::EXPLORER_WIDTH
    + contextual_theme::layout::CONTENT_MAX_WIDTH
    + 2.0 * contextual_theme::space::S8;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsGroup {
    Foundry,
    Models,
    Exclusions,
    OutputFormat,
    Appearance,
}

impl SettingsGroup {
    const ALL: [Self; 5] = [
        Self::Foundry,
        Self::Models,
        Self::Exclusions,
        Self::OutputFormat,
        Self::Appearance,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Foundry => "Azure AI Foundry",
            Self::Models => "Models",
            Self::Exclusions => "Exclusions",
            Self::OutputFormat => "Output format",
            Self::Appearance => "Appearance",
        }
    }
}

struct Field {
    label: &'static str,
    hint: &'static str,
    input: Entity<InputState>,
}

/// One of several models to route between, as the form edits it.
struct ModelRow {
    /// Stable identity for element ids while rows are added and removed.
    key: usize,
    /// Keyboard focus of the model's row in the list.
    focus: FocusHandle,
    /// Refresh the list when its displayed values or sort order change.
    _list_subs: Vec<Subscription>,
    name: Field,
    deployment: Field,
    model_name: Field,
    tier: Field,
    description: Field,
    context_limit: Field,
    max_tokens: Field,
    in_price: Field,
    cached_price: Field,
    out_price: Field,
    tokens_per_minute: Field,
}

fn text_field(
    label: &'static str,
    hint: &'static str,
    value: String,
    window: &mut Window,
    cx: &mut Context<SettingsScreen>,
) -> Field {
    Field {
        label,
        hint,
        input: cx.new(|cx| InputState::new(window, cx).default_value(value)),
    }
}

fn model_row(
    key: usize,
    m: &ModelSettings,
    window: &mut Window,
    cx: &mut Context<SettingsScreen>,
) -> ModelRow {
    const PRICE: &str = "USD. Empty: no cost is shown for this model.";
    let name = text_field(
        "Name",
        "Short name, e.g. terra. Used by the routing model and the model control.",
        m.name.clone(),
        window,
        cx,
    );
    let deployment = text_field(
        "Deployment",
        "Sent as the model name on every request.",
        opt(m.deployment.clone()),
        window,
        cx,
    );
    let model_name = text_field(
        "Model name",
        "The underlying model. Empty: the deployment name.",
        opt(m.model_name.clone()),
        window,
        cx,
    );
    let tier = text_field(
        "Tier",
        "Required. 1 or higher; higher is more capable.",
        opt(m.tier),
        window,
        cx,
    );
    let list_subs = [&name, &deployment, &model_name, &tier]
        .into_iter()
        .map(|field| cx.observe(&field.input, |_, _, cx| cx.notify()))
        .collect();
    ModelRow {
        key,
        focus: cx.focus_handle(),
        _list_subs: list_subs,
        name,
        deployment,
        model_name,
        tier,
        description: text_field(
            "Good for",
            "A sentence the routing model chooses from.",
            opt(m.description.clone()),
            window,
            cx,
        ),
        context_limit: text_field(
            "Context limit",
            "Input tokens the model accepts. Empty: 128000.",
            opt(m.context_limit),
            window,
            cx,
        ),
        max_tokens: text_field(
            "Max output tokens",
            "Per reply. Empty: 16384.",
            opt(m.max_completion_tokens),
            window,
            cx,
        ),
        in_price: text_field(
            "Input price / 1M",
            PRICE,
            opt(m.input_price_per_1m),
            window,
            cx,
        ),
        cached_price: text_field(
            "Cached input price / 1M",
            "USD. Empty: cached input is charged like other input.",
            opt(m.cached_input_price_per_1m),
            window,
            cx,
        ),
        out_price: text_field(
            "Output price / 1M",
            PRICE,
            opt(m.output_price_per_1m),
            window,
            cx,
        ),
        tokens_per_minute: text_field(
            "Tokens per minute",
            "This deployment's quota; requests wait until they fit. Empty: no pacing.",
            opt(m.tokens_per_minute),
            window,
            cx,
        ),
    }
}

pub struct SettingsScreen {
    app: Entity<AppModel>,
    active_group: SettingsGroup,
    group_focus: [FocusHandle; 5],
    endpoint: Field,
    api_key: Field,
    api_version: Field,
    deployment: Field,
    model_name: Field,
    context_limit: Field,
    max_tokens: Field,
    in_price: Field,
    out_price: Field,
    tokens_per_minute: Field,
    /// Several models to route between (replacing the single deployment above).
    models: Vec<ModelRow>,
    next_model_key: usize,
    /// The model that chooses (a row's key).
    router_key: Option<usize>,
    /// The model whose form is shown (a row's key).
    selected_model: Option<usize>,
    max_kb: Field,
    new_exclusion: Entity<InputState>,
    output_prompt: Entity<TextareaState>,
    exclusions: Vec<String>,
    presets: Entity<PresetsEditor>,
    respect_gitignore: bool,
    skip_binary: bool,
    include_hidden: bool,
    reduce_motion: bool,
    motion_focus: FocusHandle,
    /// Remove the stored key on save.
    remove_key: bool,
    /// Starting file or folder for the env-file picker.
    env_path: Entity<InputState>,
    /// An API key read from an env file, stored on Save (unless one was typed).
    imported_key: Option<String>,
    status: Option<String>,
    /// The env-file picker, while open.
    env_dialog: Option<(Entity<EnvFileDialog>, Subscription)>,
    _sub: Subscription,
}

fn opt<T: ToString>(v: Option<T>) -> String {
    v.map(|v| v.to_string()).unwrap_or_default()
}

/// Sort valid tiers numerically, with missing or invalid tiers last.
fn tier_sort_key(tier: Option<&str>) -> (bool, Option<u8>) {
    let tier = tier
        .and_then(|t| t.parse::<u8>().ok())
        .filter(|&t| t > 0);
    (tier.is_none(), tier)
}

impl SettingsScreen {
    pub fn new(app: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let a = app.read(cx);
        let s = a.settings.clone();
        let has_key = a.has_api_key();
        let f = &s.foundry;
        let mut field = |label, hint, value: String, masked: bool| Field {
            label,
            hint,
            input: cx.new(|cx| {
                let st = InputState::new(window, cx).default_value(value);
                if masked {
                    st.masked(true).placeholder(if has_key {
                        "Stored ••••••••  (type to replace)"
                    } else {
                        "Paste the key"
                    })
                } else {
                    st
                }
            }),
        };
        let endpoint = field(
            "Endpoint",
            "https://<resource>.openai.azure.com/",
            opt(f.endpoint.clone()),
            false,
        );
        let api_key = field(
            "API key",
            "Kept in the OS keychain, never in settings.json.",
            String::new(),
            true,
        );
        let api_version = field(
            "API version",
            "For chat completions, e.g. 2025-04-01-preview.",
            opt(f.api_version.clone()),
            false,
        );
        let deployment = field(
            "Deployment",
            "Sent as the model name on every request.",
            opt(f.deployment.clone()),
            false,
        );
        let model_name = field(
            "Model name",
            "The underlying model. Empty: the deployment name.",
            opt(f.model_name.clone()),
            false,
        );
        let context_limit = field(
            "Context limit",
            "Input tokens the model accepts. Empty: 128000.",
            opt(f.context_limit),
            false,
        );
        let max_tokens = field(
            "Max output tokens",
            "Per reply.",
            opt(f.max_completion_tokens),
            false,
        );
        let in_price = field(
            "Input price / 1M",
            "USD. Empty: no cost is shown.",
            opt(f.input_price_per_1m),
            false,
        );
        let out_price = field(
            "Output price / 1M",
            "USD. Empty: no cost is shown.",
            opt(f.output_price_per_1m),
            false,
        );
        let tokens_per_minute = field(
            "Tokens per minute",
            "The deployment's quota; requests wait until they fit. Empty: no pacing.",
            opt(f.tokens_per_minute),
            false,
        );
        let max_kb = field(
            "Max file size (KB)",
            "Larger files are listed as too large.",
            s.workspace.max_file_size_kb.to_string(),
            false,
        );
        let models: Vec<ModelRow> = f
            .models
            .iter()
            .enumerate()
            .map(|(key, m)| model_row(key, m, window, cx))
            .collect();
        let router_key = f.router.as_deref().and_then(|router| {
            f.models
                .iter()
                .position(|m| m.name.trim().eq_ignore_ascii_case(router.trim()))
        });
        let new_exclusion =
            cx.new(|cx| InputState::new(window, cx).placeholder("Add a glob, e.g. **/*.min.js"));
        let output_prompt = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(10)
                .default_value(s.workspace.output_prompt.clone())
        });
        let env_guess = find_env_local(&app.read(cx).state.recents);
        let env_path = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Path to a .env file or folder")
                .default_value(env_guess)
        });
        let detected = app
            .read(cx)
            .project
            .as_ref()
            .map(|p| p.read(cx).frameworks.clone())
            .unwrap_or_default();
        let presets = cx.new(|_| PresetsEditor::new(s.workspace.presets.clone(), detected));
        let sub = cx.observe(&app, |_, _, cx| cx.notify());
        if cfg!(debug_assertions)
            && std::env::var("CONTEXTUAL_DEV_OPEN").as_deref() == Ok("envfile")
        {
            cx.defer_in(window, |this: &mut Self, window, cx| {
                this.browse_env(window, cx)
            });
        }
        Self {
            // `CONTEXTUAL_DEV_OPEN=models` with `--settings`: start on the Models group.
            active_group: if cfg!(debug_assertions)
                && std::env::var("CONTEXTUAL_DEV_OPEN").as_deref() == Ok("models")
            {
                SettingsGroup::Models
            } else {
                SettingsGroup::Foundry
            },
            group_focus: SettingsGroup::ALL.map(|_| cx.focus_handle()),
            endpoint,
            api_key,
            api_version,
            deployment,
            model_name,
            context_limit,
            max_tokens,
            in_price,
            out_price,
            tokens_per_minute,
            next_model_key: models.len(),
            selected_model: models.first().map(|m| m.key),
            models,
            router_key,
            max_kb,
            new_exclusion,
            output_prompt,
            exclusions: s.workspace.exclusions.clone(),
            presets,
            respect_gitignore: s.workspace.respect_gitignore,
            skip_binary: s.workspace.skip_binary,
            include_hidden: s.workspace.include_hidden,
            reduce_motion: s.reduce_motion,
            motion_focus: cx.focus_handle(),
            remove_key: false,
            env_path,
            imported_key: None,
            status: None,
            env_dialog: None,
            app,
            _sub: sub,
        }
    }

    fn text(&self, f: &Field, cx: &Context<Self>) -> Option<String> {
        let v = f.input.read(cx).value().trim().to_string();
        (!v.is_empty()).then_some(v)
    }

    /// Builds settings from the form; `Err` names the first invalid number.
    fn collect(&self, cx: &Context<Self>) -> Result<(Settings, Option<String>), String> {
        fn num<T: std::str::FromStr>(v: Option<String>, label: &str) -> Result<Option<T>, String> {
            v.map(|s| {
                s.parse::<T>()
                    .map_err(|_| format!("{label} must be a number."))
            })
            .transpose()
        }
        let mut s = self.app.read(cx).settings.clone();
        s.foundry = FoundrySettings {
            endpoint: self.text(&self.endpoint, cx),
            api_version: self.text(&self.api_version, cx),
            deployment: self.text(&self.deployment, cx),
            model_name: self.text(&self.model_name, cx),
            max_completion_tokens: num(self.text(&self.max_tokens, cx), self.max_tokens.label)?,
            context_limit: num(self.text(&self.context_limit, cx), self.context_limit.label)?,
            input_price_per_1m: num(self.text(&self.in_price, cx), self.in_price.label)?,
            output_price_per_1m: num(self.text(&self.out_price, cx), self.out_price.label)?,
            tokens_per_minute: num(
                self.text(&self.tokens_per_minute, cx),
                self.tokens_per_minute.label,
            )?,
            models: self.models_from_form(true, cx)?,
            router: self.router_name(cx),
        };
        s.workspace.exclusions = self.exclusions.clone();
        s.workspace.presets = self.presets.read(cx).presets(cx);
        s.workspace.max_file_size_kb =
            num(self.text(&self.max_kb, cx), self.max_kb.label)?.unwrap_or(256.0);
        s.workspace.respect_gitignore = self.respect_gitignore;
        s.workspace.skip_binary = self.skip_binary;
        s.workspace.include_hidden = self.include_hidden;
        s.workspace.output_prompt = self.output_prompt.read(cx).value().to_string();
        s.reduce_motion = self.reduce_motion;
        // Appearance is saved immediately; keep the latest value, including title-bar changes.
        let key = if self.remove_key {
            Some(String::new())
        } else {
            self.text(&self.api_key, cx)
                .or_else(|| self.imported_key.clone())
        };
        Ok((s, key))
    }

    /// The name typed for the routing model, lower case.
    fn router_name(&self, cx: &Context<Self>) -> Option<String> {
        let row = self
            .models
            .iter()
            .find(|m| Some(m.key) == self.router_key)?;
        self.text(&row.name, cx).map(|name| name.to_lowercase())
    }

    /// The models as typed. `strict`: a model without a name, a name used twice and a value
    /// that is not a number are errors; otherwise such models and values are left out.
    fn models_from_form(
        &self,
        strict: bool,
        cx: &Context<Self>,
    ) -> Result<Vec<ModelSettings>, String> {
        fn num<T: std::str::FromStr>(
            v: Option<String>,
            what: &str,
            name: &str,
            strict: bool,
        ) -> Result<Option<T>, String> {
            match v.map(|s| s.parse::<T>()) {
                None => Ok(None),
                Some(Ok(n)) => Ok(Some(n)),
                Some(Err(_)) if strict => Err(format!("{what} of {name} must be a number.")),
                Some(Err(_)) => Ok(None),
            }
        }
        let mut models: Vec<ModelSettings> = Vec::new();
        for row in &self.models {
            let name = match self.text(&row.name, cx) {
                Some(name) => name.to_lowercase(),
                None if strict => return Err("Every model needs a name.".into()),
                None => continue,
            };
            if models.iter().any(|m| m.name == name) {
                if strict {
                    return Err(format!("Two models are named {name}."));
                }
                continue;
            }
            let value = |f: &Field| self.text(f, cx);
            let tier = match num::<u8>(value(&row.tier), "The tier", &name, strict)? {
                Some(0) if strict => {
                    return Err(format!("The tier of {name} must be 1 or higher."));
                }
                Some(0) => None,
                tier => tier,
            };
            models.push(ModelSettings {
                deployment: value(&row.deployment),
                model_name: value(&row.model_name),
                tier,
                description: value(&row.description),
                context_limit: num(
                    value(&row.context_limit),
                    "The context limit",
                    &name,
                    strict,
                )?,
                max_completion_tokens: num(
                    value(&row.max_tokens),
                    "The max output tokens",
                    &name,
                    strict,
                )?,
                input_price_per_1m: num(value(&row.in_price), "The input price", &name, strict)?,
                cached_input_price_per_1m: num(
                    value(&row.cached_price),
                    "The cached input price",
                    &name,
                    strict,
                )?,
                output_price_per_1m: num(value(&row.out_price), "The output price", &name, strict)?,
                tokens_per_minute: num(
                    value(&row.tokens_per_minute),
                    "The tokens per minute",
                    &name,
                    strict,
                )?,
                name,
            });
        }
        Ok(models)
    }

    /// Replaces the list (after an import), keeping the selection and routing model by name.
    fn set_models(
        &mut self,
        models: &[ModelSettings],
        router: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected = self
            .models
            .iter()
            .find(|m| Some(m.key) == self.selected_model)
            .and_then(|row| self.text(&row.name, cx))
            .map(|name| name.to_lowercase());
        let first = self.next_model_key;
        self.models = models
            .iter()
            .enumerate()
            .map(|(i, m)| model_row(first + i, m, window, cx))
            .collect();
        self.next_model_key = first + models.len();
        let key_of = |name: Option<&str>| {
            let name = name?.trim();
            models
                .iter()
                .position(|m| m.name.trim().eq_ignore_ascii_case(name))
                .map(|i| first + i)
        };
        self.router_key = key_of(router);
        self.selected_model =
            key_of(selected.as_deref()).or_else(|| self.models.first().map(|m| m.key));
    }

    /// Adds a model and shows its form. The first model becomes the routing model.
    fn add_model(&mut self, preset: ModelSettings, window: &mut Window, cx: &mut Context<Self>) {
        let key = self.next_model_key;
        self.next_model_key += 1;
        self.models.push(model_row(key, &preset, window, cx));
        self.selected_model = Some(key);
        if self.router_key.is_none() {
            self.router_key = Some(key);
        }
        cx.notify();
    }

    fn remove_model(&mut self, key: usize, cx: &mut Context<Self>) {
        let Some(at) = self.models.iter().position(|m| m.key == key) else {
            return;
        };
        self.models.remove(at);
        // The neighbour takes the selection; the routing model is not replaced silently.
        if self.selected_model == Some(key) {
            self.selected_model = self
                .models
                .get(at)
                .or_else(|| self.models.last())
                .map(|m| m.key);
        }
        if self.router_key == Some(key) {
            self.router_key = None;
        }
        cx.notify();
    }

    fn select_model(&mut self, key: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.selected_model = Some(key);
        // Move focus off an input of the form that is about to be replaced.
        if let Some(row) = self.models.iter().find(|m| m.key == key) {
            row.focus.focus(window, cx);
        }
        cx.notify();
    }

    /// Opens the env-file picker at the path typed so far.
    fn browse_env(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let start = self.env_path.read(cx).value().trim().to_string();
        let dialog = cx.new(|cx| EnvFileDialog::new(Some(start), window, cx));
        let sub = cx.subscribe_in(&dialog, window, |this, _, e: &EnvFileEvent, window, cx| {
            if let EnvFileEvent::Picked(path) = e {
                let path = path.clone();
                this.env_path
                    .update(cx, |i, cx| i.set_value(path, window, cx));
                this.import_env(window, cx);
            }
            this.env_dialog = None;
            cx.notify();
        });
        self.env_dialog = Some((dialog, sub));
        cx.notify();
    }

    /// Fills the Foundry fields from an env file (review, then Save).
    fn import_env(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = self.env_path.read(cx).value().trim().to_string();
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                self.status = Some(format!("Couldn't read {path}: {e}"));
                cx.notify();
                return;
            }
        };
        let imp = contextual_core::migrate::import_env(&text);
        let f = &imp.foundry;
        let mut n = 0;
        for (field, value) in [
            (&self.endpoint, f.endpoint.clone()),
            (&self.api_version, f.api_version.clone()),
            (&self.deployment, f.deployment.clone()),
            (&self.model_name, f.model_name.clone()),
            (&self.context_limit, f.context_limit.map(|v| v.to_string())),
            (
                &self.max_tokens,
                f.max_completion_tokens.map(|v| v.to_string()),
            ),
            (&self.in_price, f.input_price_per_1m.map(|v| v.to_string())),
            (
                &self.out_price,
                f.output_price_per_1m.map(|v| v.to_string()),
            ),
            (
                &self.tokens_per_minute,
                f.tokens_per_minute.map(|v| v.to_string()),
            ),
        ] {
            if let Some(v) = value {
                field.input.update(cx, |i, cx| i.set_value(v, window, cx));
                n += 1;
            }
        }
        if imp.api_key.is_some() {
            n += 1;
        }
        self.imported_key = imp.api_key;
        // Models are matched by name: the file updates the ones it names and adds new ones.
        let named = f.models.len();
        if named > 0 || f.router.is_some() {
            let mut current = FoundrySettings {
                models: self.models_from_form(false, cx).unwrap_or_default(),
                router: self.router_name(cx),
                ..FoundrySettings::default()
            };
            contextual_core::migrate::merge(
                &mut current,
                &FoundrySettings {
                    models: f.models.clone(),
                    router: f.router.clone(),
                    ..FoundrySettings::default()
                },
            );
            self.set_models(&current.models, current.router.as_deref(), window, cx);
        }
        let mut msg = format!(
            "Imported {n} values{} from the env file. Review them, then use the save icon in the header (the API key goes to the keychain).",
            match named {
                0 => String::new(),
                1 => " and 1 model".into(),
                n => format!(" and {n} models"),
            }
        );
        if !imp.skipped.is_empty() {
            msg.push_str(&format!(
                " Skipped (not numbers): {}.",
                imp.skipped.join(", ")
            ));
        }
        self.status = Some(msg);
        cx.notify();
    }

    fn persist(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let result = self
            .collect(cx)
            .and_then(|(s, key)| self.app.update(cx, |a, cx| a.try_save_settings(s, key, cx)));
        match result {
            Ok(()) => {
                self.api_key
                    .input
                    .update(cx, |i, cx| i.set_value("", window, cx));
                self.remove_key = false;
                self.imported_key = None;
                self.status = Some("Saved.".into());
                cx.notify();
                true
            }
            Err(e) => {
                self.status = Some(e);
                cx.notify();
                false
            }
        }
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.persist(window, cx) {
            self.app.update(cx, |a, cx| a.finish_settings(cx));
        }
    }

    /// The general list and the framework presets back to the defaults (saved on Save).
    fn reset_exclusions(&mut self, cx: &mut Context<Self>) {
        let mut w = contextual_core::settings::WorkspaceSettings::default();
        w.reset_exclusions();
        self.exclusions = w.exclusions;
        self.presets.update(cx, |p, cx| p.set(w.presets, cx));
        cx.notify();
    }

    fn set_theme(&mut self, theme: ThemePreference, window: &mut Window, cx: &mut Context<Self>) {
        crate::theme::apply(crate::theme::resolve(theme, window), Some(window), cx);
        self.app.update(cx, |a, cx| {
            let mut s = a.settings.clone();
            s.theme = theme;
            a.save_settings(s, None, cx);
        });
        cx.notify();
    }

    fn select_group(&mut self, group: SettingsGroup, window: &mut Window, cx: &mut Context<Self>) {
        self.active_group = group;
        // Move focus off any input that is about to be unmounted.
        self.group_focus[group as usize].focus(window, cx);
        cx.notify();
    }

    fn navigation(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let c = &cx.tokens().colors;
        div()
            .id("settings-navigation")
            .flex()
            .flex_col()
            .flex_none()
            .w(px(contextual_theme::layout::EXPLORER_WIDTH))
            .min_h_0()
            .overflow_y_scroll()
            .px(sp(4))
            .py(sp(6))
            .gap(sp(1))
            .border_r_1()
            .border_color(hsla(c.border))
            .bg(hsla(c.surface_000))
            .type_style(contextual_theme::type_style::UI_BODY)
            .children(SettingsGroup::ALL.iter().copied().map(|group| {
                let active = self.active_group == group;
                div()
                    .id(("settings-group", group as usize))
                    .track_focus(&self.group_focus[group as usize])
                    .tab_index(0)
                    .relative()
                    .flex()
                    .flex_none()
                    .items_center()
                    .h(control_height())
                    .px(sp(3))
                    .rounded(px(contextual_theme::radius::SM))
                    .border_1()
                    .border_color(hsla(contextual_theme::Rgba {
                        a: 0.0,
                        ..c.focus_ring
                    }))
                    .bg(hsla(if active { c.surface_100 } else { c.surface_000 }))
                    .text_color(hsla(if active { c.ink_strong } else { c.ink_muted }))
                    .cursor_pointer()
                    .when(!active, |d| {
                        d.hover(|s| s.bg(hsla(c.surface_hover)).text_color(hsla(c.ink)))
                    })
                    .focus(|s| s.border_color(hsla(c.focus_ring)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.select_group(group, window, cx);
                    }))
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        let index = group as usize;
                        let count = SettingsGroup::ALL.len();
                        let next = match event.keystroke.key.as_str() {
                            "enter" | "space" => group,
                            "up" => SettingsGroup::ALL[(index + count - 1) % count],
                            "down" => SettingsGroup::ALL[(index + 1) % count],
                            "home" => SettingsGroup::Foundry,
                            "end" => SettingsGroup::Appearance,
                            _ => return,
                        };
                        cx.stop_propagation();
                        this.select_group(next, window, cx);
                    }))
                    .child(group.label())
                    .when(active, |d| {
                        d.child(
                            div()
                                .absolute()
                                .left_0()
                                .top(sp(1))
                                .bottom(sp(1))
                                .border_l_2()
                                .border_color(hsla(c.accent)),
                        )
                    })
            }))
    }

    fn field_row(&self, f: &Field, cx: &Context<Self>) -> impl IntoElement + use<> {
        let c = &cx.tokens().colors;
        div()
            .flex()
            .items_start()
            .gap(sp(4))
            .py(sp(2))
            .child(
                div()
                    .w(px(LABEL_W))
                    .flex_none()
                    .pt(px(3.0))
                    .child(div().text_color(hsla(c.ink)).child(f.label))
                    .child(
                        div()
                            .type_style(contextual_theme::type_style::UI_SMALL)
                            .text_color(hsla(c.ink_muted))
                            .child(f.hint),
                    ),
            )
            .child(div().w(px(FIELD_W)).child(Input::new(&f.input)))
    }

    fn switch_row(
        &self,
        id: &'static str,
        label: &'static str,
        on: bool,
        toggle: fn(&mut Self),
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        div()
            .flex()
            .items_center()
            .gap(sp(2))
            .py(sp(1))
            .child(
                Switch::new(id, on).on_toggle(cx.listener(move |t, _, _, cx| {
                    toggle(t);
                    cx.notify();
                })),
            )
            .child(label)
    }
}

fn section(title: &'static str, cx: &gpui_kit::App) -> impl IntoElement + use<> {
    let c = &cx.tokens().colors;
    div()
        .mt(sp(6))
        .mb(sp(2))
        .pb(sp(2))
        .border_b_1()
        .border_color(hsla(c.border))
        .type_style(contextual_theme::type_style::UI_TITLE)
        .text_color(hsla(c.ink_strong))
        .child(title)
}

/// "GPT-6 Astra: 922k context · $10 in / $1 cached / $50 out per 1M · above 272k: $20 / $75"
fn catalog_line(m: &contextual_core::models::ModelInfo) -> String {
    use contextual_core::logic::format::fmt_k;
    let usd = |v: f64| format!("${v}");
    let mut line = format!("{}: {} context", m.label, fmt_k(m.context_limit));
    if let (Some(i), Some(o)) = (m.input_price_per_1m, m.output_price_per_1m) {
        line.push_str(&format!(" · {} in", usd(i)));
        if let Some(cached) = m.cached_input_price_per_1m {
            line.push_str(&format!(" / {} cached", usd(cached)));
        }
        line.push_str(&format!(" / {} out per 1M", usd(o)));
    }
    if let Some(l) = &m.long_context {
        line.push_str(&format!(
            " · above {} input: {} / {}",
            fmt_k(l.above_input_tokens),
            usd(l.input_price_per_1m),
            usd(l.output_price_per_1m)
        ));
    }
    line
}

fn subsection(
    title: &'static str,
    hint: &'static str,
    cx: &gpui_kit::App,
) -> impl IntoElement + use<> {
    let c = &cx.tokens().colors;
    div()
        .pt(sp(3))
        .child(div().text_color(hsla(c.ink_strong)).child(title))
        .child(
            div()
                .type_style(contextual_theme::type_style::UI_SMALL)
                .text_color(hsla(c.ink_muted))
                .child(hint),
        )
}

impl Render for SettingsScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = &cx.tokens().colors;
        let a = self.app.read(cx);
        let theme = a.settings.theme;
        let test = a.connection_test.clone();
        let persistent = a.secrets.persistent();
        let has_key = a.has_api_key();
        let env_note = [
            "AZURE_OPENAI_ENDPOINT",
            "AZURE_OPENAI_API_KEY",
            "AZURE_OPENAI_DEPLOYMENT",
        ]
        .iter()
        .any(|k| std::env::var(k).is_ok());
        let test_line = match &test {
            ConnectionTest::Idle => None,
            ConnectionTest::Running => Some(("Testing…".to_string(), c.ink_muted)),
            ConnectionTest::Ok { latency_ms } => {
                Some((format!("Connected in {latency_ms} ms."), c.success))
            }
            ConnectionTest::Failed(e) => Some((e.clone(), c.danger)),
        };
        let catalog_note = {
            let typed = |f: &Field| f.input.read(cx).value().trim().to_string();
            let mut names = vec![typed(&self.model_name), typed(&self.deployment)];
            if let Some(cfg) = &a.config {
                names.extend([cfg.model_name.clone(), cfg.deployment.clone()]);
            }
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            // Only when the user described this model under "models" in settings.json.
            contextual_core::models::find(&a.settings.models, &names).map(|m| {
                format!(
                    "{}. From \"models\" in settings.json; fields below override it.",
                    catalog_line(m)
                )
            })
        };
        let frameworks_note = a.project.as_ref().map(|p| {
            let p = p.read(cx);
            if p.frameworks.is_empty() {
                format!("No framework detected in {}.", p.name)
            } else {
                format!("Detected in {}: {}.", p.name, p.frameworks.join(", "))
            }
        });
        let chips = self
            .exclusions
            .iter()
            .enumerate()
            .map(|(i, g)| {
                div()
                    .flex()
                    .items_center()
                    .gap(sp(1))
                    .pl(sp(2))
                    .rounded(px(contextual_theme::radius::SM))
                    .bg(hsla(c.surface_300))
                    .type_style(contextual_theme::type_style::CODE_SMALL)
                    .child(SharedString::from(g.clone()))
                    .child(
                        div()
                            .id(("rm-glob", i))
                            .px(sp(1))
                            .cursor_pointer()
                            .text_color(hsla(c.ink_muted))
                            .hover(|s| s.text_color(hsla(c.ink)))
                            .on_click(cx.listener(move |t, _, _, cx| {
                                t.exclusions.remove(i);
                                cx.notify();
                            }))
                            .child(Icon::new(IconName::Close).size(px(12.0))),
                    )
            })
            .collect::<Vec<_>>();
        let theme_buttons = [
            (ThemePreference::Dark, "Dark"),
            (ThemePreference::Light, "Light"),
            (ThemePreference::Moss, "Moss"),
            (ThemePreference::System, "System"),
        ]
        .into_iter()
        .map(|(t, label)| {
            Button::new(label, label)
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::Xs)
                .selected(theme == t)
                .on_click(cx.listener(move |this, _, w, cx| this.set_theme(t, w, cx)))
        })
        .collect::<Vec<_>>();

        let routing_problem = match &a.foundry {
            FoundryStatus::Unconfigured { missing, invalid }
                if !a.settings.foundry.models.is_empty() =>
            {
                Some(
                    ConfigProblems {
                        missing: missing.clone(),
                        invalid: invalid.clone(),
                    }
                    .message()
                    .replace(" See .env.example.", ""),
                )
            }
            _ => None,
        };
        let typed = |f: &Field| f.input.read(cx).value().trim().to_string();
        let catalog_of = |row: &ModelRow| {
            let names = [typed(&row.model_name), typed(&row.deployment)];
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            contextual_core::models::find(&a.settings.models, &names)
        };
        // Sort only the displayed rows, leaving saved order and row identities intact.
        let mut sorted_models: Vec<_> = self
            .models
            .iter()
            .map(|row| {
                let tier = Some(typed(&row.tier))
                    .filter(|t| !t.is_empty())
                    .or_else(|| catalog_of(row).and_then(|m| m.tier).map(|t| t.to_string()));
                (row, tier)
            })
            .collect();
        sorted_models.sort_by_key(|(_, tier)| tier_sort_key(tier.as_deref()));
        // One row per model: name, what it is, its tier, and which one chooses.
        let model_list = sorted_models
            .into_iter()
            .map(|(row, tier)| {
                let key = row.key;
                let active = self.selected_model == Some(key);
                let catalog = catalog_of(row);
                let name = typed(&row.name);
                let what = catalog
                    .map(|m| m.label.clone())
                    .or_else(|| Some(typed(&row.deployment)).filter(|d| !d.is_empty()))
                    .unwrap_or_default();
                div()
                    .id(("model-row", key))
                    .track_focus(&row.focus)
                    .tab_index(0)
                    .flex()
                    .items_center()
                    .gap(sp(3))
                    .h(control_height())
                    .px(sp(3))
                    .rounded(px(contextual_theme::radius::SM))
                    .border_1()
                    .border_color(hsla(contextual_theme::Rgba {
                        a: 0.0,
                        ..c.focus_ring
                    }))
                    .when(active, |d| d.bg(hsla(c.surface_selected)))
                    .when(!active, |d| d.hover(|s| s.bg(hsla(c.surface_hover))))
                    .focus(|s| s.border_color(hsla(c.focus_ring)))
                    .cursor_pointer()
                    .on_click(cx.listener(move |t, _, window, cx| t.select_model(key, window, cx)))
                    .on_key_down(cx.listener(move |t, e: &KeyDownEvent, window, cx| {
                        if matches!(e.keystroke.key.as_str(), "enter" | "space") {
                            cx.stop_propagation();
                            t.select_model(key, window, cx);
                        }
                    }))
                    .child(
                        div()
                            .flex_none()
                            .text_color(hsla(if name.is_empty() {
                                c.ink_muted
                            } else {
                                c.ink_strong
                            }))
                            .child(SharedString::from(if name.is_empty() {
                                "New model".to_string()
                            } else {
                                name
                            })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .type_style(contextual_theme::type_style::UI_SMALL)
                            .text_color(hsla(c.ink_muted))
                            .child(SharedString::from(what)),
                    )
                    .when(self.router_key == Some(key), |d| {
                        d.child(Tag::new("Routing model").tone(TagTone::Skill))
                    })
                    .child(
                        div()
                            .flex_none()
                            .type_style(contextual_theme::type_style::UI_SMALL)
                            .text_color(hsla(if tier.is_some() {
                                c.ink_muted
                            } else {
                                c.warning
                            }))
                            .child(SharedString::from(match tier {
                                Some(t) => format!("Tier {t}"),
                                None => "No tier".to_string(),
                            })),
                    )
            })
            .collect::<Vec<_>>();
        // The form of the selected model.
        let model_form = self
            .models
            .iter()
            .find(|m| Some(m.key) == self.selected_model)
            .map(|row| {
                let key = row.key;
                // Only when the user described this model under "models" in settings.json.
                let note = catalog_of(row).map(|m| {
                    format!(
                        "{}{}. From \"models\" in settings.json; fields below override it.",
                        catalog_line(m),
                        m.tier.map(|t| format!(" · tier {t}")).unwrap_or_default()
                    )
                });
                let is_router = self.router_key == Some(key);
                div()
                    .child(self.field_row(&row.name, cx))
                    .child(self.field_row(&row.deployment, cx))
                    .child(self.field_row(&row.model_name, cx))
                    .when_some(note, |d, note| {
                        d.child(
                            div()
                                .pl(px(LABEL_W) + sp(4))
                                .pb(sp(1))
                                .type_style(contextual_theme::type_style::UI_SMALL)
                                .text_color(hsla(c.ink_muted))
                                .child(SharedString::from(note)),
                        )
                    })
                    .child(self.field_row(&row.tier, cx))
                    .child(self.field_row(&row.description, cx))
                    .child(self.field_row(&row.context_limit, cx))
                    .child(self.field_row(&row.max_tokens, cx))
                    .child(self.field_row(&row.in_price, cx))
                    .child(self.field_row(&row.cached_price, cx))
                    .child(self.field_row(&row.out_price, cx))
                    .child(self.field_row(&row.tokens_per_minute, cx))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(sp(3))
                            .pt(sp(2))
                            .pl(px(LABEL_W) + sp(4))
                            .type_style(contextual_theme::type_style::UI_SMALL)
                            .text_color(hsla(c.ink_muted))
                            .child(if is_router {
                                div()
                                    .child("This model reads each message first and chooses.")
                                    .into_any_element()
                            } else {
                                Button::new("make-router", "Make routing model")
                                    .on_click(cx.listener(move |t, _, _, cx| {
                                        t.router_key = Some(key);
                                        cx.notify();
                                    }))
                                    .into_any_element()
                            })
                            .child(LinkButton::new(
                                ("remove-model", key),
                                "Remove model",
                                cx.listener(move |t, _, _, cx| t.remove_model(key, cx)),
                            )),
                    )
            });
        let no_models = self.models.is_empty();
        let no_router = !no_models && self.router_key.is_none();

        let env_dialog = self.env_dialog.as_ref().map(|(d, _)| d.clone());
        let header = div()
            .flex()
            .flex_none()
            .justify_center()
            .px(sp(8))
            .py(sp(3))
            .border_b_1()
            .border_color(hsla(c.border))
            .bg(hsla(c.surface_100))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(sp(3))
                    .w_full()
                    .max_w(px(SETTINGS_MAX_W))
                    .child(
                        IconButton::new("back", IconName::ArrowLeft, "Back").on_click(cx.listener(
                            |t, _, _, cx| t.app.update(cx, |a, cx| a.close_settings(cx)),
                        )),
                    )
                    .child(
                        div()
                            .flex_1()
                            .type_style(contextual_theme::type_style::UI_TITLE)
                            .text_color(hsla(c.ink_strong))
                            .child("Settings"),
                    )
                    .child(
                        IconButton::new("save", IconName::Check, "Save settings and return")
                            .on_click(cx.listener(|t, _, w, cx| t.save(w, cx))),
                    ),
            );
        let navigation = self.navigation(cx);
        let active_group = self.active_group;
        let page = div()
            .id(("settings", active_group as usize))
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .items_center()
            .overflow_y_scroll()
            .overflow_x_scroll()
            .px(sp(8))
            .bg(hsla(c.surface_100))
            .type_style(contextual_theme::type_style::UI_BODY)
            .child(
                div()
                    .w_full()
                    .min_w(px(LABEL_W + FIELD_W) + sp(4))
                    .max_w(px(contextual_theme::layout::CONTENT_MAX_WIDTH))
                    .flex_none()
                    .pb(sp(8))
                    .child(section(active_group.label(), cx))
                    .when(active_group == SettingsGroup::Foundry, |d| {
                        d.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(sp(2))
                            .mb(sp(2))
                            .type_style(contextual_theme::type_style::UI_SMALL)
                            .text_color(hsla(c.ink_muted))
                            .child("Env file or folder")
                            .child(div().flex_1().min_w_0().type_style(contextual_theme::type_style::CODE_SMALL).child(Input::new(&self.env_path)))
                            .child(Button::new("browse-env", "Import .env file…").on_click(cx.listener(|t, _, w, cx| t.browse_env(w, cx)))),
                    )
                    .child(
                        div()
                            .mb(sp(2))
                            .type_style(contextual_theme::type_style::UI_SMALL)
                            .text_color(hsla(c.ink_muted))
                            .child("Choose a .env or .env.local file to fill the connection fields. Review the values before saving."),
                    )
                    .when(self.imported_key.is_some(), |d| {
                        d.child(
                            div()
                                .pl(px(LABEL_W) + sp(4))
                                .type_style(contextual_theme::type_style::UI_SMALL)
                                .text_color(hsla(c.ink_muted))
                                .child("An API key was imported and will be stored on Save."),
                        )
                    })
                    .when(env_note, |d| {
                        d.child(
                            div()
                                .mb(sp(2))
                                .type_style(contextual_theme::type_style::UI_SMALL)
                                .text_color(hsla(c.ink_muted))
                                .child("Environment variables (AZURE_OPENAI_*) are set and override these values."),
                        )
                    })
                    .child(self.field_row(&self.endpoint, cx))
                    .child(self.field_row(&self.api_key, cx))
                    .child(
                        div()
                            .flex()
                            .gap(sp(3))
                            .pl(px(LABEL_W) + sp(4))
                            .type_style(contextual_theme::type_style::UI_SMALL)
                            .text_color(hsla(if persistent { c.ink_muted } else { c.warning }))
                            .child(if persistent {
                                "Stored in the OS keychain."
                            } else {
                                "No OS keychain found: the key is kept until the app quits. Set AZURE_OPENAI_API_KEY to avoid re-entering it."
                            })
                            .when(has_key, |d| {
                                d.child(LinkButton::new("remove-key", "Remove stored key", cx.listener(|t, _, w, cx| {
                                    t.remove_key = true;
                                    t.persist(w, cx);
                                })))
                            }),
                    )
                    .child(self.field_row(&self.api_version, cx))
                    .child(self.field_row(&self.deployment, cx))
                    .child(self.field_row(&self.model_name, cx))
                    .when_some(catalog_note, |d, note| {
                        d.child(
                            div()
                                .pl(px(LABEL_W) + sp(4))
                                .pb(sp(1))
                                .type_style(contextual_theme::type_style::UI_SMALL)
                                .text_color(hsla(c.ink_muted))
                                .child(SharedString::from(note)),
                        )
                    })
                    .child(self.field_row(&self.context_limit, cx))
                    .child(self.field_row(&self.max_tokens, cx))
                    .child(self.field_row(&self.in_price, cx))
                    .child(self.field_row(&self.out_price, cx))
                    .child(self.field_row(&self.tokens_per_minute, cx))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(sp(3))
                            .pt(sp(2))
                            .pl(px(LABEL_W) + sp(4))
                            .child(
                                Button::new("test", "Test connection")
                                    .disabled(test == ConnectionTest::Running)
                                    .on_click(cx.listener(|t, _, _, cx| t.app.update(cx, |a, cx| a.test_connection(cx)))),
                            )
                            .when_some(test_line, |d, (text, color)| {
                                d.child(div().flex_1().type_style(contextual_theme::type_style::UI_SMALL).text_color(hsla(color)).child(SharedString::from(text)))
                            }),
                    )
                    .child(
                        div()
                            .pl(px(LABEL_W) + sp(4))
                            .pt(sp(1))
                            .type_style(contextual_theme::type_style::UI_SMALL)
                            .text_color(hsla(c.ink_muted))
                            .child("Test uses the saved settings: save first."),
                    )
                    .when(!no_models, |d| {
                        d.child(
                            div()
                                .pt(sp(3))
                                .type_style(contextual_theme::type_style::UI_SMALL)
                                .text_color(hsla(c.ink_muted))
                                .child("Models are defined under Models: the deployment, model name, limits and prices on this page are not used."),
                        )
                    })
                    })
                    .when(active_group == SettingsGroup::Models, |d| {
                        d.child(
                            div()
                                .type_style(contextual_theme::type_style::UI_SMALL)
                                .text_color(hsla(c.ink_muted))
                                .child("With two or more models, the routing model reads each message first and chooses the model that answers it. Models share the endpoint, API key and API version under Azure AI Foundry and replace its single deployment."),
                        )
                        .when_some(routing_problem, |d, problem| {
                            d.child(
                                div()
                                    .pt(sp(2))
                                    .type_style(contextual_theme::type_style::UI_SMALL)
                                    .text_color(hsla(c.warning))
                                    .child(SharedString::from(problem)),
                            )
                        })
                        .when(no_router, |d| {
                            d.child(
                                div()
                                    .pt(sp(2))
                                    .type_style(contextual_theme::type_style::UI_SMALL)
                                    .text_color(hsla(c.warning))
                                    .child("No routing model: select a model and press Make routing model."),
                            )
                        })
                        .when(no_models, |d| {
                            d.child(
                                div()
                                    .pt(sp(3))
                                    .text_color(hsla(c.ink_muted))
                                    .child("No models defined. Import an env file that names them, or add one by hand."),
                            )
                        })
                        .when(!no_models, |d| {
                            d.child(
                                div()
                                    .mt(sp(3))
                                    .p(sp(1))
                                    .flex()
                                    .flex_col()
                                    .rounded(px(contextual_theme::radius::MD))
                                    .border_1()
                                    .border_color(hsla(c.border))
                                    .children(model_list),
                            )
                        })
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap(sp(2))
                                .pt(sp(3))
                                .child(
                                    Button::new("add-model", "Add model").on_click(cx.listener(
                                        |t, _, w, cx| t.add_model(ModelSettings::default(), w, cx),
                                    )),
                                )
                                .child(
                                    Button::new("models-import-env", "Import .env file…")
                                        .on_click(cx.listener(|t, _, w, cx| t.browse_env(w, cx))),
                                ),
                        )
                        .when_some(model_form, |d, form| {
                            d.child(subsection(
                                "Selected model",
                                "Saved with the rest of the settings.",
                                cx,
                            ))
                            .child(form)
                        })
                    })
                    .when(active_group == SettingsGroup::Exclusions, |d| {
                        d.child(subsection("All projects", "Left out of every project.", cx))
                    .child(div().flex().flex_wrap().gap(sp(1)).py(sp(2)).children(chips))
                    .child(
                        div()
                            .flex()
                            .gap(sp(2))
                            .w(px(FIELD_W))
                            .child(div().flex_1().child(Input::new(&self.new_exclusion)))
                            .child(Button::new("add-glob", "Add").on_click(cx.listener(|t, _, w, cx| {
                                let g = t.new_exclusion.read(cx).value().trim().to_string();
                                if !g.is_empty() && !t.exclusions.contains(&g) {
                                    t.exclusions.push(g);
                                    t.new_exclusion.update(cx, |i, cx| i.set_value("", w, cx));
                                    cx.notify();
                                }
                            }))),
                    )
                    .child(subsection("Frameworks", "Applied to a project that contains one of the marker files.", cx))
                    .when_some(frameworks_note, |d, n| {
                        d.child(div().pb(sp(1)).type_style(contextual_theme::type_style::UI_SMALL).text_color(hsla(c.ink_muted)).child(SharedString::from(n)))
                    })
                    .child(self.presets.clone())
                    .child(
                        div().pt(sp(2)).pb(sp(2)).type_style(contextual_theme::type_style::UI_SMALL).child(LinkButton::new(
                            "reset-exclusions",
                            "Reset exclusions to defaults",
                            cx.listener(|t, _, _, cx| t.reset_exclusions(cx)),
                        )),
                    )
                    .child(self.field_row(&self.max_kb, cx))
                    .child(self.switch_row("respect", "Respect .gitignore", self.respect_gitignore, |t| t.respect_gitignore = !t.respect_gitignore, cx))
                    .child(self.switch_row("binary", "Skip binary files", self.skip_binary, |t| t.skip_binary = !t.skip_binary, cx))
                    .child(self.switch_row("hidden", "Include hidden files", self.include_hidden, |t| t.include_hidden = !t.include_hidden, cx))
                    })
                    .when(active_group == SettingsGroup::OutputFormat, |d| {
                        d.child(
                        div()
                            .mb(sp(2))
                            .type_style(contextual_theme::type_style::UI_SMALL)
                            .text_color(hsla(c.ink_muted))
                            .child("System prompt that tells the model how to return changes. The edit format is appended automatically."),
                    )
                    .child(div().type_style(contextual_theme::type_style::CODE_SMALL).child(Textarea::new(&self.output_prompt)))
                    })
                    .when(active_group == SettingsGroup::Appearance, |d| {
                        d.child(div().flex().gap(sp(1)).children(theme_buttons))
                    .child(
                        div()
                            .id("reduce-motion-focus")
                            .mt(sp(3))
                            .p(sp(1))
                            .track_focus(&self.motion_focus)
                            .tab_index(0)
                            .rounded(px(contextual_theme::radius::SM))
                            .border_1()
                            .border_color(hsla(c.surface_100))
                            .focus(|s| s.border_color(hsla(c.focus_ring)))
                            .on_key_down(cx.listener(|t, e: &KeyDownEvent, _, cx| {
                                if matches!(e.keystroke.key.as_str(), "enter" | "space") {
                                    t.reduce_motion = !t.reduce_motion;
                                    cx.stop_propagation();
                                    cx.notify();
                                }
                            }))
                            .child(self.switch_row(
                                "reduce-motion",
                                "Reduce motion",
                                self.reduce_motion,
                                |t| t.reduce_motion = !t.reduce_motion,
                                cx,
                            ))
                            .child(
                                div()
                                    .type_style(contextual_theme::type_style::UI_SMALL)
                                    .text_color(hsla(c.ink_muted))
                                    .child(
                                        "Use static activity indicators. Takes effect when you save settings.",
                                    ),
                            ),
                    )
                    }),
            );
        div()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .child(header)
            .when_some(self.status.clone(), |d, status| {
                d.child(
                    div()
                        .flex_none()
                        .px(sp(8))
                        .py(sp(2))
                        .border_b_1()
                        .border_color(hsla(c.border))
                        .bg(hsla(c.surface_100))
                        .type_style(contextual_theme::type_style::UI_SMALL)
                        .text_color(hsla(c.ink))
                        .child(SharedString::from(status)),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .justify_center()
                    .bg(hsla(c.surface_100))
                    .child(
                        div()
                            .flex()
                            .w_full()
                            .max_w(px(SETTINGS_MAX_W))
                            .min_h_0()
                            .min_w_0()
                            .child(navigation)
                            .child(page),
                    ),
            )
            .when_some(env_dialog, |d, dialog| d.child(dialog))
    }
}

/// The old app's `.env.local` near a recently opened project (the old repo is often a parent of
/// the projects opened from it), else empty.
fn find_env_local(recents: &[contextual_core::recents::RecentProject]) -> String {
    for r in recents
        .iter()
        .filter(|r| r.location == contextual_core::location::Location::Local)
    {
        let mut dir = Some(std::path::Path::new(&r.path));
        for _ in 0..4 {
            let Some(d) = dir else { break };
            let candidate = d.join(".env.local");
            if candidate.is_file() {
                return candidate.to_string_lossy().into_owned();
            }
            dir = d.parent();
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::tier_sort_key;

    #[test]
    fn tiers_sort_numerically_with_missing_and_invalid_last() {
        let mut rows = [
            ("missing", None),
            ("ten", Some("10")),
            ("invalid", Some("high")),
            ("two", Some("2")),
            ("zero", Some("0")),
            ("also two", Some("2")),
        ];
        rows.sort_by_key(|(_, tier)| tier_sort_key(*tier));
        assert_eq!(
            rows.map(|(name, _)| name),
            ["two", "also two", "ten", "missing", "invalid", "zero"]
        );
    }
}
