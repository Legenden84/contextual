//! Phase 1 spike: one window, a nav of the six gate prototypes, and a theme switch.

mod code_view;
mod composer;
mod controls;
mod diff_view;
mod file_tree;
mod sample;
mod stream;

use contextual_theme::{Appearance, layout};
use gpui_kit::{
    AnyView, AppContext, Context, Entity, FocusHandle, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, Styled, Window, div, px,
};

use crate::ToggleTheme;
use crate::theme::{ActiveTheme, TypeExt, hsla, sp};
use crate::ui::{Button, ButtonSize, ButtonVariant, pane_header};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Proto {
    Tree,
    Code,
    Diff,
    Composer,
    Stream,
    Controls,
}

impl Proto {
    const ALL: [Proto; 6] = [
        Proto::Tree,
        Proto::Code,
        Proto::Diff,
        Proto::Composer,
        Proto::Stream,
        Proto::Controls,
    ];

    /// `CONTEXTUAL_SPIKE=tree|code|diff|composer|stream|controls` picks the start view (screenshots).
    fn from_env() -> Option<Proto> {
        let v = std::env::var("CONTEXTUAL_SPIKE").ok()?;
        Proto::ALL.into_iter().find(|p| p.key() == v)
    }

    fn key(self) -> &'static str {
        match self {
            Proto::Tree => "tree",
            Proto::Code => "code",
            Proto::Diff => "diff",
            Proto::Composer => "composer",
            Proto::Stream => "stream",
            Proto::Controls => "controls",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Proto::Tree => "1. File tree (20,000 rows)",
            Proto::Code => "2. Code view (5,000 lines)",
            Proto::Diff => "3. Side-by-side diff",
            Proto::Composer => "4. Composer",
            Proto::Stream => "5. Streaming markdown",
            Proto::Controls => "6. Dropdown, dialog, toast",
        }
    }
}

pub struct Spike {
    active: Proto,
    focus: FocusHandle,
    appearance: Appearance,
    tree: Entity<file_tree::FileTreeProto>,
    code: Entity<code_view::CodeViewProto>,
    diff: Entity<diff_view::DiffProto>,
    composer: Entity<composer::ComposerProto>,
    stream: Entity<stream::StreamProto>,
    controls: Entity<controls::ControlsProto>,
}

impl Spike {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let appearance = match std::env::var("CONTEXTUAL_THEME").as_deref() {
            Ok("light") => Appearance::Light,
            _ => Appearance::Dark,
        };
        if appearance != Appearance::Dark {
            crate::theme::apply(appearance, Some(window), cx);
        }
        let this = Self {
            active: Proto::from_env().unwrap_or(Proto::Tree),
            focus,
            appearance,
            tree: cx.new(file_tree::FileTreeProto::new),
            code: cx.new(|cx| code_view::CodeViewProto::new(window, cx)),
            diff: cx.new(diff_view::DiffProto::new),
            composer: cx.new(|cx| composer::ComposerProto::new(window, cx)),
            stream: cx.new(stream::StreamProto::new),
            controls: cx.new(|cx| controls::ControlsProto::new(window, cx)),
        };
        if this.active == Proto::Stream {
            this.stream.update(cx, |s, cx| s.restart(cx));
        }
        this
    }

    fn set_appearance(
        &mut self,
        appearance: Appearance,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.appearance = appearance;
        crate::theme::apply(appearance, Some(window), cx);
        cx.notify();
    }

    fn active_view(&self) -> AnyView {
        match self.active {
            Proto::Tree => self.tree.clone().into(),
            Proto::Code => self.code.clone().into(),
            Proto::Diff => self.diff.clone().into(),
            Proto::Composer => self.composer.clone().into(),
            Proto::Stream => self.stream.clone().into(),
            Proto::Controls => self.controls.clone().into(),
        }
    }
}

impl Render for Spike {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = &cx.tokens().colors;
        let nav = Proto::ALL.iter().enumerate().map(|(i, &p)| {
            Button::new(("proto", i), p.label())
                .variant(ButtonVariant::Ghost)
                .selected(self.active == p)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.active = p;
                    if p == Proto::Stream {
                        this.stream.update(cx, |s, cx| s.restart(cx));
                    }
                    cx.notify();
                }))
        });
        let theme_switch = div()
            .flex()
            .gap(sp(1))
            .child(
                Button::new("dark", "Dark")
                    .size(ButtonSize::Xs)
                    .variant(ButtonVariant::Ghost)
                    .selected(self.appearance == Appearance::Dark)
                    .on_click(
                        cx.listener(|this, _, w, cx| this.set_appearance(Appearance::Dark, w, cx)),
                    ),
            )
            .child(
                Button::new("light", "Light")
                    .size(ButtonSize::Xs)
                    .variant(ButtonVariant::Ghost)
                    .selected(self.appearance == Appearance::Light)
                    .on_click(
                        cx.listener(|this, _, w, cx| this.set_appearance(Appearance::Light, w, cx)),
                    ),
            );

        div()
            .size_full()
            .flex()
            .bg(hsla(c.surface_100))
            .text_color(hsla(c.ink))
            .type_style(contextual_theme::type_style::UI_BODY)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &ToggleTheme, w, cx| {
                let next = if this.appearance == Appearance::Dark {
                    Appearance::Light
                } else {
                    Appearance::Dark
                };
                this.set_appearance(next, w, cx)
            }))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .w(px(layout::EXPLORER_WIDTH))
                    .h_full()
                    .bg(hsla(c.surface_000))
                    .border_r_1()
                    .border_color(hsla(c.border))
                    .child(pane_header("Phase 1 spike", theme_switch, cx))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(sp(1))
                            .p(sp(2))
                            .children(nav.map(|b| div().flex().child(b))),
                    )
                    .child(
                        div()
                            .mt_auto()
                            .p(sp(3))
                            .type_style(contextual_theme::type_style::UI_SMALL)
                            .text_color(hsla(c.ink_muted))
                            .child(SharedString::from(format!(
                                "gpui-kit 0.9.0 · gpui-pre 0.3.11 · {}",
                                std::env::consts::OS
                            ))),
                    ),
            )
            .child(div().flex_1().h_full().min_w_0().child(self.active_view()))
    }
}
