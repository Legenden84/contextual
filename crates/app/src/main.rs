//! Contextual, the native desktop app (Windows, macOS).
//!
//! Phase 1 of `docs/plans/cross-platform-rust-port.md`: a spike window with the six prototypes
//! the go/no-go gate judges (tree, code view, diff, composer, streaming markdown, controls).

mod assets;
mod spike;
mod theme;
mod ui;

use contextual_theme::Appearance;
use gpui_kit::{
    App, AppContext, Bounds, KeyBinding, Menu, MenuItem, TitlebarOptions, WindowBounds,
    WindowOptions, px, size,
};

gpui_kit::actions!(contextual, [Quit, ToggleTheme]);

fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx: &mut App| {
            gpui_kit::init(cx);
            assets::load_fonts(cx);
            theme::apply(Appearance::Dark, None, cx);
            keymap(cx);

            let bounds = Bounds::centered(None, size(px(1280.), px(820.)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Contextual".into()),
                    ..Default::default()
                }),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| spike::Spike::new(window, cx))
            })
            .expect("failed to open the main window");
            cx.activate(true);
        });
}

fn keymap(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| cx.quit());
    // Per-OS modifier (plan §6.4): cmd on macOS, ctrl on Windows (and Linux, for development).
    #[cfg(target_os = "macos")]
    const KEYS: [&str; 2] = ["cmd-q", "cmd-shift-t"];
    #[cfg(not(target_os = "macos"))]
    const KEYS: [&str; 2] = ["ctrl-q", "ctrl-shift-t"];
    cx.bind_keys([
        KeyBinding::new(KEYS[0], Quit, None),
        KeyBinding::new(KEYS[1], ToggleTheme, None),
    ]);
    cx.set_menus([Menu {
        name: "Contextual".into(),
        items: vec![
            MenuItem::action("Toggle theme", ToggleTheme),
            MenuItem::separator(),
            MenuItem::action("Quit", Quit),
        ],
        disabled: false,
    }]);
}
