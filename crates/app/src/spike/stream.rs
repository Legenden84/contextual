//! Prototype 5: a ~50 KB reply streamed into a native markdown view, appended live in small
//! deltas like the Responses API sends them.

use std::time::{Duration, Instant};

use gpui_kit::component::text::{TextView, TextViewState};
use gpui_kit::{
    AppContext, Context, Entity, IntoElement, ParentElement, Render, SharedString, Styled, Task,
    Window, div, px,
};

use crate::theme::{ActiveTheme, TypeExt, hsla, sp};

/// Bytes per delta and delay between deltas (~2,000 tokens/s, faster than real models).
const DELTA: usize = 96;
const TICK: Duration = Duration::from_millis(8);

pub struct StreamProto {
    view: Entity<TextViewState>,
    sent: usize,
    total: usize,
    started: Instant,
    finished: Option<Duration>,
    task: Option<Task<()>>,
}

impl StreamProto {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let view = cx.new(|cx| {
            TextViewState::markdown("", cx)
                .selectable(true)
                .scrollable(true)
        });
        Self {
            view,
            sent: 0,
            total: 0,
            started: Instant::now(),
            finished: None,
            task: None,
        }
    }

    pub fn restart(&mut self, cx: &mut Context<Self>) {
        let reply = super::sample::long_reply();
        self.total = reply.len();
        self.sent = 0;
        self.started = Instant::now();
        self.finished = None;
        self.view.update(cx, |v, cx| v.set_text("", cx));
        self.task = Some(cx.spawn(async move |this, cx| {
            let mut pos = 0;
            while pos < reply.len() {
                cx.background_executor().timer(TICK).await;
                let mut end = (pos + DELTA).min(reply.len());
                while !reply.is_char_boundary(end) {
                    end += 1;
                }
                let chunk = reply[pos..end].to_string();
                pos = end;
                let alive = this
                    .update(cx, |this, cx| {
                        this.view.update(cx, |v, cx| v.push_str(&chunk, cx));
                        this.sent = pos;
                        cx.notify();
                    })
                    .is_ok();
                if !alive {
                    return;
                }
            }
            let _ = this.update(cx, |this, cx| {
                this.finished = Some(this.started.elapsed());
                cx.notify();
            });
        }));
    }
}

impl Render for StreamProto {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = &cx.tokens().colors;
        let status = match self.finished {
            Some(d) => format!("{} KB in {:.1}s", self.total / 1024, d.as_secs_f32()),
            None => format!("streaming… {} / {} KB", self.sent / 1024, self.total / 1024),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .child(
                div()
                    .w_full()
                    .flex()
                    .justify_end()
                    .px(sp(3))
                    .py(sp(1))
                    .type_style(contextual_theme::type_style::UI_SMALL)
                    .text_color(hsla(c.ink_muted))
                    .child(SharedString::from(status)),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .max_w(px(contextual_theme::layout::CONTENT_MAX_WIDTH))
                    .px(sp(4))
                    .type_style(contextual_theme::type_style::UI_PROSE)
                    .child(TextView::new(&self.view)),
            )
    }
}
