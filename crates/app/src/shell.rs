//! The window's root view: the current screen, the status bar and the Open folder dialog.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext, Context, Entity, EntityId, FocusHandle, InteractiveElement, IntoElement,
    ParentElement, Render, Styled, Subscription, Window, div,
};

use crate::screens::settings::SettingsScreen;
use crate::screens::welcome::WelcomeScreen;
use crate::screens::workspace::WorkspaceScreen;
use crate::state::{AppModel, Screen};
use crate::theme::{ActiveTheme, TypeExt, hsla};
use crate::views::folder_dialog::FolderDialog;
use crate::views::status_bar::status_bar;
use crate::views::title_bar::title_bar;
use crate::{CloseFolder, OpenFolder, OpenSettings, ToggleTheme};

pub struct Shell {
    app: Entity<AppModel>,
    focus: FocusHandle,
    welcome: Entity<WelcomeScreen>,
    /// Rebuilt when another project opens.
    workspace: Option<(EntityId, Entity<WorkspaceScreen>)>,
    /// Rebuilt each time Settings opens, so the form starts from the saved values.
    settings: Option<Entity<SettingsScreen>>,
    folder_dialog: Option<Entity<FolderDialog>>,
    /// The window title, also shown in the title bar.
    title: String,
    _sub: Subscription,
}

impl Shell {
    pub fn new(app: Entity<AppModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let sub = cx.observe_in(&app, window, |this: &mut Self, _, window, cx| {
            this.sync(window, cx)
        });
        let mut this = Self {
            welcome: cx.new(|cx| WelcomeScreen::new(app.clone(), cx)),
            app,
            focus,
            workspace: None,
            settings: None,
            folder_dialog: None,
            title: "Contextual".into(),
            _sub: sub,
        };
        this.sync(window, cx);
        this
    }

    /// Creates or drops screen views to match the model.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let a = self.app.read(cx);
        let screen = a.screen;
        let project = a.project.clone();
        let dialog_open = a.folder_dialog_open;
        let title = match (screen, &project) {
            (Screen::Settings, _) => "Settings — Contextual".to_string(),
            (Screen::Workspace, Some(p)) => format!("{} — Contextual", p.read(cx).name),
            _ => "Contextual".to_string(),
        };
        window.set_window_title(&title);
        self.title = title;

        match project {
            Some(p) if self.workspace.as_ref().map(|(id, _)| *id) != Some(p.entity_id()) => {
                let app = self.app.clone();
                let view = cx.new(|cx| WorkspaceScreen::new(app, p.clone(), window, cx));
                self.workspace = Some((p.entity_id(), view));
            }
            None => self.workspace = None,
            _ => {}
        }
        if screen == Screen::Settings {
            if self.settings.is_none() {
                let app = self.app.clone();
                self.settings = Some(cx.new(|cx| SettingsScreen::new(app, window, cx)));
            }
        } else {
            self.settings = None;
        }
        if dialog_open && self.folder_dialog.is_none() {
            let app = self.app.clone();
            let start = self
                .app
                .read(cx)
                .state
                .recents
                .first()
                .map(|r| (r.path.clone(), r.location.clone()));
            self.folder_dialog = Some(cx.new(|cx| FolderDialog::new(app, start, window, cx)));
        } else if !dialog_open && self.folder_dialog.is_some() {
            self.folder_dialog = None;
            window.focus(&self.focus, cx);
        }
        cx.notify();
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = &cx.tokens().colors;
        let screen = self.app.read(cx).screen;
        let body = match screen {
            Screen::Settings => self.settings.clone().map(|s| s.into_any_element()),
            Screen::Workspace => self
                .workspace
                .as_ref()
                .map(|(_, w)| w.clone().into_any_element()),
            Screen::Welcome => None,
        }
        .unwrap_or_else(|| self.welcome.clone().into_any_element());

        div()
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(hsla(c.surface_100))
            .text_color(hsla(c.ink))
            .type_style(contextual_theme::type_style::UI_BODY)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &OpenFolder, _, cx| {
                this.app.update(cx, |a, cx| {
                    a.folder_dialog_open = true;
                    cx.notify();
                })
            }))
            .on_action(cx.listener(|this, _: &CloseFolder, _, cx| {
                this.app.update(cx, |a, cx| a.close_project(cx))
            }))
            .on_action(cx.listener(|this, _: &OpenSettings, _, cx| {
                this.app.update(cx, |a, cx| a.open_settings(cx))
            }))
            .on_action(cx.listener(|this, _: &ToggleTheme, window, cx| {
                use contextual_core::settings::ThemePreference;
                use contextual_theme::Appearance;

                let next = cx.tokens().appearance.next();
                crate::theme::apply(next, Some(window), cx);
                this.app.update(cx, |a, cx| {
                    let mut s = a.settings.clone();
                    s.theme = match next {
                        Appearance::Dark => ThemePreference::Dark,
                        Appearance::Light => ThemePreference::Light,
                        Appearance::Moss => ThemePreference::Moss,
                    };
                    a.save_settings(s, None, cx);
                });
            }))
            // On Linux the compositor may draw its own title bar (X11, or Wayland with server-side
            // decorations); then ours would be a second one.
            .when(
                !(cfg!(target_os = "linux")
                    && matches!(window.window_decorations(), gpui_kit::Decorations::Server)),
                |d| d.child(title_bar(&self.title, cx)),
            )
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(body)
                    .when_some(self.folder_dialog.clone(), |d, f| d.child(f)),
            )
            .child(status_bar(&self.app, cx))
    }
}
