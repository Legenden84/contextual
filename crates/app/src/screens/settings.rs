//! Settings (design spec: ScreenSettings): the Azure AI Foundry connection (API key in the OS
//! keychain), Test connection, exclusions, the output-format prompt and the theme.

use contextual_core::settings::{FoundrySettings, Settings, ThemePreference};
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::{Icon, IconName};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div, px,
};

use crate::state::{AppModel, ConnectionTest};
use crate::theme::{ActiveTheme, TypeExt, hsla, sp};
use crate::ui::{Button, ButtonSize, ButtonVariant, LinkButton, Switch};
use crate::views::presets_editor::PresetsEditor;

const LABEL_W: f32 = 200.0;
const FIELD_W: f32 = 420.0;

struct Field {
    label: &'static str,
    hint: &'static str,
    input: Entity<InputState>,
}

pub struct SettingsScreen {
    app: Entity<AppModel>,
    endpoint: Field,
    api_key: Field,
    api_version: Field,
    deployment: Field,
    model_name: Field,
    context_limit: Field,
    max_tokens: Field,
    in_price: Field,
    out_price: Field,
    max_kb: Field,
    new_exclusion: Entity<InputState>,
    output_prompt: Entity<TextareaState>,
    exclusions: Vec<String>,
    presets: Entity<PresetsEditor>,
    respect_gitignore: bool,
    skip_binary: bool,
    include_hidden: bool,
    theme: ThemePreference,
    /// Remove the stored key on save.
    remove_key: bool,
    /// `.env.local` of the old app to import from.
    env_path: Entity<InputState>,
    /// An API key read from `.env.local`, stored on Save (unless one was typed).
    imported_key: Option<String>,
    status: Option<String>,
    _sub: Subscription,
}

fn opt<T: ToString>(v: Option<T>) -> String {
    v.map(|v| v.to_string()).unwrap_or_default()
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
            "Shown in the UI; looked up in the model catalog.",
            opt(f.model_name.clone()),
            false,
        );
        let context_limit = field(
            "Context limit",
            "Tokens; empty: from the model catalog.",
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
            "USD; empty: from the model catalog.",
            opt(f.input_price_per_1m),
            false,
        );
        let out_price = field(
            "Output price / 1M",
            "USD; empty: from the model catalog.",
            opt(f.output_price_per_1m),
            false,
        );
        let max_kb = field(
            "Max file size (KB)",
            "Larger files are listed as too large.",
            s.workspace.max_file_size_kb.to_string(),
            false,
        );
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
                .placeholder("/path/to/old/contextual/.env.local")
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
        Self {
            endpoint,
            api_key,
            api_version,
            deployment,
            model_name,
            context_limit,
            max_tokens,
            in_price,
            out_price,
            max_kb,
            new_exclusion,
            output_prompt,
            exclusions: s.workspace.exclusions.clone(),
            presets,
            respect_gitignore: s.workspace.respect_gitignore,
            skip_binary: s.workspace.skip_binary,
            include_hidden: s.workspace.include_hidden,
            theme: s.theme,
            remove_key: false,
            env_path,
            imported_key: None,
            status: None,
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
        };
        s.workspace.exclusions = self.exclusions.clone();
        s.workspace.presets = self.presets.read(cx).presets(cx);
        s.workspace.max_file_size_kb =
            num(self.text(&self.max_kb, cx), self.max_kb.label)?.unwrap_or(256.0);
        s.workspace.respect_gitignore = self.respect_gitignore;
        s.workspace.skip_binary = self.skip_binary;
        s.workspace.include_hidden = self.include_hidden;
        s.workspace.output_prompt = self.output_prompt.read(cx).value().to_string();
        s.theme = self.theme;
        let key = if self.remove_key {
            Some(String::new())
        } else {
            self.text(&self.api_key, cx)
                .or_else(|| self.imported_key.clone())
        };
        Ok((s, key))
    }

    /// Fills the Foundry fields from the old app's `.env.local` (review, then Save).
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
        let mut msg = format!(
            "Imported {n} values from .env.local. Review them, then Save (the API key goes to the keychain)."
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

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.collect(cx) {
            Ok((s, key)) => {
                let typed_key = key.as_ref().is_some_and(|k| !k.is_empty());
                self.app.update(cx, |a, cx| a.save_settings(s, key, cx));
                if typed_key || self.remove_key {
                    self.api_key
                        .input
                        .update(cx, |i, cx| i.set_value("", window, cx));
                }
                self.remove_key = false;
                self.status = Some("Saved.".into());
            }
            Err(e) => self.status = Some(e),
        }
        cx.notify();
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
        self.theme = theme;
        crate::theme::apply(crate::theme::resolve(theme, window), Some(window), cx);
        self.app.update(cx, |a, cx| {
            let mut s = a.settings.clone();
            s.theme = theme;
            a.save_settings(s, None, cx);
        });
        cx.notify();
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

/// "GPT-6 Astra: 922k context · $10 in / $1 cached / $50 out per 1M · above 272k: $20 / $75."
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
    line.push_str(". From the model catalog; fields below override it.");
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
            match contextual_core::models::find(&a.settings.models, &names) {
                Some(m) => catalog_line(m),
                None => "Not in the model catalog: set the context limit and prices below, or add the model to \"models\" in settings.json.".to_string(),
            }
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
            (ThemePreference::System, "System"),
        ]
        .into_iter()
        .map(|(t, label)| {
            Button::new(label, label)
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::Xs)
                .selected(self.theme == t)
                .on_click(cx.listener(move |this, _, w, cx| this.set_theme(t, w, cx)))
        })
        .collect::<Vec<_>>();

        div()
            .id("settings")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .items_center()
            .overflow_y_scroll()
            .bg(hsla(c.surface_100))
            .type_style(contextual_theme::type_style::UI_BODY)
            .child(
                div()
                    .w(px(contextual_theme::layout::CONTENT_MAX_WIDTH))
                    .py(sp(8))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(sp(3))
                            .child(div().flex_1().type_style(contextual_theme::type_style::UI_DISPLAY).text_color(hsla(c.ink_strong)).child("Settings"))
                            .child(Button::new("back", "Back").on_click(cx.listener(|t, _, _, cx| t.app.update(cx, |a, cx| a.close_settings(cx)))))
                            .child(Button::new("save", "Save").variant(ButtonVariant::Primary).on_click(cx.listener(|t, _, w, cx| t.save(w, cx)))),
                    )
                    .when_some(self.status.clone(), |d, s| {
                        d.child(div().mt(sp(2)).type_style(contextual_theme::type_style::UI_SMALL).text_color(hsla(c.ink_muted)).child(SharedString::from(s)))
                    })
                    .child(section("Azure AI Foundry", cx))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(sp(2))
                            .mb(sp(2))
                            .type_style(contextual_theme::type_style::UI_SMALL)
                            .text_color(hsla(c.ink_muted))
                            .child("Import from the old app:")
                            .child(div().flex_1().child(Input::new(&self.env_path)))
                            .child(Button::new("import-env", "Import .env.local").on_click(cx.listener(|t, _, w, cx| t.import_env(w, cx)))),
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
                                    t.save(w, cx)
                                })))
                            }),
                    )
                    .child(self.field_row(&self.api_version, cx))
                    .child(self.field_row(&self.deployment, cx))
                    .child(self.field_row(&self.model_name, cx))
                    .child(
                        div()
                            .pl(px(LABEL_W) + sp(4))
                            .pb(sp(1))
                            .type_style(contextual_theme::type_style::UI_SMALL)
                            .text_color(hsla(c.ink_muted))
                            .child(SharedString::from(catalog_note)),
                    )
                    .child(self.field_row(&self.context_limit, cx))
                    .child(self.field_row(&self.max_tokens, cx))
                    .child(self.field_row(&self.in_price, cx))
                    .child(self.field_row(&self.out_price, cx))
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
                    .child(section("Exclusions", cx))
                    .child(subsection("All projects", "Left out of every project.", cx))
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
                    .child(section("Output format", cx))
                    .child(
                        div()
                            .mb(sp(2))
                            .type_style(contextual_theme::type_style::UI_SMALL)
                            .text_color(hsla(c.ink_muted))
                            .child("System prompt that tells the model how to return changes. The edit format is appended automatically."),
                    )
                    .child(div().type_style(contextual_theme::type_style::CODE_SMALL).child(Textarea::new(&self.output_prompt)))
                    .child(section("Appearance", cx))
                    .child(div().flex().gap(sp(1)).children(theme_buttons)),
            )
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
