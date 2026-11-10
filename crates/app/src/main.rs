//! Contextual, the native desktop app (Windows, macOS; developed on Linux).
//!
//! Normal launches start on Welcome. `contextual PATH` (or a `contextual://open?path=…` link)
//! explicitly opens PATH. `--no-restore` remains accepted for compatibility.
//! `--settings` starts on Settings. `contextual --spike` shows the Phase 1 prototypes instead.

mod assets;
mod dev;
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
    [
        Quit,
        ToggleTheme,
        OpenFolder,
        OpenSettings,
        CloseFolder,
        ZoomIn,
        ZoomOut,
        ResetZoom,
        ComposerIndent,
        ComposerOutdent
    ]
);

struct Args {
    spike: bool,
    /// Start on the Settings screen.
    settings: bool,
    path: Option<String>,
}

fn args() -> Args {
    let (mut spike, mut settings) = (false, false);
    let mut path = None;
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "--spike" => spike = true,
            "--no-restore" => {}
            "--settings" => settings = true,
            "--version" => {
                println!("contextual {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            _ if path.is_none() && !a.starts_with("--") => path = Some(a),
            _ => {}
        }
    }
    Args {
        spike,
        settings,
        path,
    }
}

fn main() {
    // Explicit opt-in, also available in release builds. Restrict this subscriber to
    // performance metadata rather than enabling logs containing request/file contents.
    if std::env::var("CONTEXTUAL_PERF").as_deref() == Ok("1")
        && tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::new(
                "off,contextual::performance=debug",
            ))
            .with_ansi(false)
            .with_writer(std::io::stderr)
            .try_init()
            .is_err()
    {
        eprintln!("Performance logging could not be enabled: a subscriber is already installed.");
    }
    runtime::init_local_offset();
    state::hosts::kill_children_with_app();
    let args = args();
    // `contextual://open?path=…` links (when the OS has the scheme registered) arrive here.
    let (url_tx, mut url_rx) = futures::channel::mpsc::unbounded::<String>();
    let app = gpui_kit::application().with_assets(assets::AppAssets);
    app.on_open_urls(move |urls| {
        for u in urls {
            let _ = url_tx.unbounded_send(u);
        }
    });
    app.run(move |cx: &mut App| {
        gpui_kit::init(cx);
        assets::load_fonts(cx);
        theme::register_languages();
        runtime::init(cx);
        keymap(cx);

        let bounds = Bounds::centered(None, size(px(1280.), px(820.)), cx);
        // We draw the title bar ourselves (views/title_bar.rs): a transparent system title bar
        // on macOS and Windows, client-side decorations on Linux (WSLg's Wayland draws none).
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("Contextual".into()),
                ..gpui_kit::component::TitleBar::title_bar_options()
            }),
            window_decorations: cfg!(target_os = "linux")
                .then_some(gpui_kit::WindowDecorations::Client),
            ..gpui_kit::component::TitleBar::window_options()
        };
        if args.spike {
            theme::apply(contextual_theme::Appearance::Dark, None, cx);
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| spike::Spike::new(window, cx))
            })
            .expect("failed to open the main window");
        } else {
            let model = cx.new(state::AppModel::new);
            let links = model.clone();
            cx.spawn(async move |cx| {
                use futures::StreamExt as _;
                while let Some(url) = url_rx.next().await {
                    if let Some(path) = contextual_core::location::parse_open_url(&url) {
                        links.update(cx, |m, cx| m.open_folder(path, None, cx));
                    }
                }
            })
            .detach();
            let path = args.path.clone();
            gpui_kit::open_window(options, cx, |window, cx| {
                let pref = model.read(cx).settings.theme;
                theme::apply(theme::resolve(pref, window), Some(window), cx);
                model.update(cx, |m, cx| {
                    m.startup(path, cx);
                    if args.settings {
                        m.open_settings(cx);
                    }
                });
                cx.new(|cx| shell::Shell::new(model.clone(), window, cx))
            })
            .expect("failed to open the main window");
        }
        cx.activate(true);
    });
}

/// Include both representations of shifted `+`, plus unshifted `=`. Keyboard layouts and
/// platform backends can report the shifted key differently.
fn zoom_keys(macos: bool) -> [&'static str; 6] {
    if macos {
        [
            "cmd-=",
            "cmd-+",
            "cmd-shift-=",
            "cmd-shift-+",
            "cmd--",
            "cmd-0",
        ]
    } else {
        [
            "ctrl-=",
            "ctrl-+",
            "ctrl-shift-=",
            "ctrl-shift-+",
            "ctrl--",
            "ctrl-0",
        ]
    }
}

fn keymap(cx: &mut App) {
    // Match the focused Input inside the composer, rather than competing with
    // its bindings at an ancestor node. Register after the toolkit's keymap.
    cx.bind_keys([
        KeyBinding::new(
            "tab",
            ComposerIndent,
            Some("ContextualComposer > Input"),
        ),
        KeyBinding::new(
            "shift-tab",
            ComposerOutdent,
            Some("ContextualComposer > Input"),
        ),
    ]);
    cx.on_action(|_: &Quit, cx| cx.quit());
    // Fallback for toolkit popups whose focus lives outside Shell's subtree.
    cx.on_action(|_: &ZoomIn, cx| {
        theme::set_zoom(theme::zoom_percent() as i16 + 10, cx);
    });
    cx.on_action(|_: &ZoomOut, cx| {
        theme::set_zoom(theme::zoom_percent() as i16 - 10, cx);
    });
    cx.on_action(|_: &ResetZoom, cx| {
        theme::set_zoom(100, cx);
    });
    let zoom = zoom_keys(cfg!(target_os = "macos"));
    cx.bind_keys([
        KeyBinding::new(zoom[0], ZoomIn, None),
        KeyBinding::new(zoom[1], ZoomIn, None),
        KeyBinding::new(zoom[2], ZoomIn, None),
        KeyBinding::new(zoom[3], ZoomIn, None),
        KeyBinding::new(zoom[4], ZoomOut, None),
        KeyBinding::new(zoom[5], ResetZoom, None),
    ]);
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
            MenuItem::action("Zoom in", ZoomIn),
            MenuItem::action("Zoom out", ZoomOut),
            MenuItem::action("Actual size", ResetZoom),
            MenuItem::separator(),
            MenuItem::action("Quit", Quit),
        ],
        disabled: false,
    }]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_shortcuts_use_the_native_modifier_and_parse_on_both_platforms() {
        for macos in [false, true] {
            let keys = zoom_keys(macos);
            let modifier = if macos { "cmd-" } else { "ctrl-" };
            assert!(keys.iter().all(|key| key.starts_with(modifier)));
            assert!(keys[0].ends_with('='));
            assert!(keys[1].ends_with('+'));
            assert!(keys[4].ends_with("--"));
            assert!(keys[5].ends_with("-0"));
            for key in keys {
                // Also checks the pinned GPUI parser accepts literal '+' and '-' bindings.
                let _ = KeyBinding::new(key, ZoomIn, None);
            }
        }
    }
}
