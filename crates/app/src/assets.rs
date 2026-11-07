//! Bundled fonts (SIL OFL; licences next to the files in `assets/fonts/`). Icons come from
//! `gpui-kit-assets`.

use std::borrow::Cow;

use gpui_kit::App;

const FONTS: &[&[u8]] = &[
    include_bytes!("../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../assets/fonts/Inter-SemiBold.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Bold.ttf"),
];

pub fn load_fonts(cx: &mut App) {
    let fonts = FONTS.iter().map(|f| Cow::Borrowed(*f)).collect();
    if let Err(e) = cx.text_system().add_fonts(fonts) {
        eprintln!("couldn't load bundled fonts: {e}");
    }
}

 // Icons beyond gpui-component's default set, embedded from the Lucide catalog.
gpui_kit::assets::icon_assets!(ExtraIcons, [GitBranch, Image]);

/// The default component icons plus [`ExtraIcons`].
pub struct AppAssets;

impl gpui_kit::AssetSource for AppAssets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<Cow<'static, [u8]>>> {
        match gpui_kit::assets::Assets.load(path) {
            Ok(Some(data)) => Ok(Some(data)),
            _ => ExtraIcons.load(path),
        }
    }

    fn list(&self, path: &str) -> gpui_kit::Result<Vec<gpui_kit::SharedString>> {
        let mut all = gpui_kit::assets::Assets.list(path).unwrap_or_default();
        all.extend(ExtraIcons.list(path).unwrap_or_default());
        Ok(all)
    }
}
