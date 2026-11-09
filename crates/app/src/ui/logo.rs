use std::time::Duration;

use gpui_kit::{
    Animation, AnimationExt, AnyElement, App, Div, ElementId, Hsla, IntoElement, ParentElement,
    Styled, div, svg,
};

use crate::theme::{ActiveTheme, hsla, px, sp};

/// The supplied Moss mark keeps its Moss/Algae colours even when previewed in another theme.
pub fn moss_mark(size: f32) -> impl IntoElement {
    let c = &contextual_theme::Theme::MOSS.colors;
    div()
        .flex_none()
        .size(px(size))
        .rounded_full()
        .bg(hsla(c.surface_selected))
        .child(
            svg()
                .path(crate::assets::MOSS_ICON_PATH)
                .size_full()
                .text_color(hsla(c.ink)),
        )
}

/// A fixed-size code-flow mark. Only the strokes' emphasis changes; the label and layout
/// never move. Keep `id` stable across activity changes so the loop does not restart.
/// Static mode creates no animation, and removing the mark removes its frame scheduling.
pub fn activity_mark(id: impl Into<ElementId>, animate: bool, cx: &App) -> AnyElement {
    let color = hsla(cx.tokens().colors.accent_ink);
    let mark = div()
        .flex_none()
        .size(sp(4))
        .flex()
        .flex_col()
        .justify_center()
        .gap(sp(1));

    if animate {
        mark.with_animation(
            id,
            Animation::new(Duration::from_millis(1800)).repeat(),
            move |mark, progress| mark.children(activity_strokes(Some(progress), color)),
        )
        .into_any_element()
    } else {
        mark.children(activity_strokes(None, color))
            .into_any_element()
    }
}

fn activity_strokes(progress: Option<f32>, color: Hsla) -> Vec<Div> {
    [3, 4, 2]
        .into_iter()
        .enumerate()
        .map(|(index, width)| {
            div()
                .flex_none()
                .w(sp(width))
                // Icon stroke geometry, inside the token-sized 16px glyph.
                .h(px(2.0))
                .rounded(px(contextual_theme::radius::FULL))
                .bg(color)
                .opacity(progress.map_or(1.0, |p| stroke_opacity(p, index)))
        })
        .collect()
}

/// A smooth highlight travels down the strokes, with a quiet interval between passes.
/// The periodic curve has identical value and slope at the loop boundary.
fn stroke_opacity(progress: f32, index: usize) -> f32 {
    let phase = (progress - index as f32 * 0.18) * std::f32::consts::TAU;
    let highlight = phase.cos().max(0.0).powi(2);
    0.28 + 0.72 * highlight
}

/// The Contextual mark on a 16-unit grid: an accent sheet, an outlined sheet over it, and two
/// success-coloured lines (the old app's `Logo`), drawn from the theme tokens.
pub fn logo(size: f32, cx: &App) -> impl IntoElement + use<> {
    let c = &cx.tokens().colors;
    let u = size / 16.0;
    let at = |x: f32, y: f32, w: f32, h: f32| {
        div()
            .absolute()
            .left(px(x * u))
            .top(px(y * u))
            .w(px(w * u))
            .h(px(h * u))
    };
    div()
        .relative()
        .flex_none()
        .size(px(size))
        .child(
            at(1.0, 1.5, 9.0, 11.0)
                .rounded(px(2.0 * u))
                .bg(hsla(c.accent)),
        )
        .child(
            at(6.0, 3.5, 9.0, 11.0)
                .rounded(px(2.0 * u))
                .bg(hsla(c.surface_100))
                .border(px((1.2 * u).max(1.0)))
                .border_color(hsla(c.ink_muted)),
        )
        .child(at(8.5, 6.8, 4.0, 1.4).rounded_full().bg(hsla(c.success)))
        .child(at(8.5, 9.3, 2.5, 1.4).rounded_full().bg(hsla(c.success)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_loop_is_bounded_and_seamless() {
        for index in 0..3 {
            assert!((stroke_opacity(0.0, index) - stroke_opacity(1.0, index)).abs() < 0.00001);
            for frame in 0..=180 {
                let opacity = stroke_opacity(frame as f32 / 180.0, index);
                assert!((0.28..=1.0).contains(&opacity));
            }
        }
    }

    #[test]
    fn highlight_passes_through_the_strokes_in_order() {
        for index in 0..3 {
            let progress = index as f32 * 0.18;
            assert!((stroke_opacity(progress, index) - 1.0).abs() < 0.00001);
            for other in 0..3 {
                if other != index {
                    assert!(stroke_opacity(progress, other) < stroke_opacity(progress, index));
                }
            }
        }
    }
}
