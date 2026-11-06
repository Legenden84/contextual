//! Contextual, the native desktop app (Windows, macOS; developed on Linux).
//!
//! `contextual [PATH]` opens PATH (or the last project). `contextual --spike` shows the Phase 1
//! prototypes instead.

mod assets;
mod runtime;
mod screens;
mod secrets;
mod shell;
mod spike;
mod state;
mod theme;
mod ui;
mod views;

use gpui_kit::{
    App, AppContext, Bounds, KeyBinding, Menu, MenuItem, TitlebarOptions, WindowBounds,
    WindowOptions, px, size,
};

gpui_kit::actions!(
    contextual,
    [Quit, ToggleTheme, OpenFolder, OpenSettings, CloseFolder]
);

struct Args {
    spike: bool,
    path: Option<String>,
}

fn args() -> Args {
    let mut spike = false;
    let mut path = None;
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "--spike" => spike = true,
            "--version" => {
                println!("contextual {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            _ if path.is_none() && !a.starts_with("--") => path = Some(a),
            _ => {}
        }
    }
    Args { spike, path }
}

fn main() {
    let args = args();
    gpui_kit::application()
        .with_assets(assets::AppAssets)
        .run(move |cx: &mut App| {
            gpui_kit::init(cx);
            assets::load_fonts(cx);
            runtime::init(cx);
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
            if args.spike {
                theme::apply(contextual_theme::Appearance::Dark, None, cx);
                gpui_kit::open_window(options, cx, |window, cx| {
                    cx.new(|cx| spike::Spike::new(window, cx))
                })
                .expect("failed to open the main window");
            } else {
                let model = cx.new(state::AppModel::new);
                let path = args.path.clone();
                gpui_kit::open_window(options, cx, |window, cx| {
                    let pref = model.read(cx).settings.theme;
                    theme::apply(theme::resolve(pref, window), Some(window), cx);
                    model.update(cx, |m, cx| m.startup(path, window, cx));
                    cx.new(|cx| shell::Shell::new(model.clone(), window, cx))
                })
                .expect("failed to open the main window");
            }
            cx.activate(true);
        });
}

fn keymap(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| cx.quit());
    // Per-OS modifier (plan §6.4): cmd on macOS, ctrl on Windows (and Linux, for development).
    #[cfg(target_os = "macos")]
    const KEYS: [&str; 4] = ["cmd-q", "cmd-shift-t", "cmd-o", "cmd-,"];
    #[cfg(not(target_os = "macos"))]
    const KEYS: [&str; 4] = ["ctrl-q", "ctrl-shift-t", "ctrl-o", "ctrl-,"];
    cx.bind_keys([
        KeyBinding::new(KEYS[0], Quit, None),
        KeyBinding::new(KEYS[1], ToggleTheme, None),
        KeyBinding::new(KEYS[2], OpenFolder, None),
        KeyBinding::new(KEYS[3], OpenSettings, None),
    ]);
    cx.set_menus([Menu {
        name: "Contextual".into(),
        items: vec![
            MenuItem::action("Open folder…", OpenFolder),
            MenuItem::action("Close folder", CloseFolder),
            MenuItem::action("Settings…", OpenSettings),
            MenuItem::action("Toggle theme", ToggleTheme),
            MenuItem::separator(),
            MenuItem::action("Quit", Quit),
        ],
        disabled: false,
    }]);
}
