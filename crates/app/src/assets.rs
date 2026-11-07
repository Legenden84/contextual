//! Bundled fonts (SIL OFL; licences next to the files in `assets/fonts/`). Standard icons come
//! from `gpui-kit-assets`; the Moss mark is a vector recreation of the supplied artwork.

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

pub const MOSS_ICON_PATH: &str = "icons/contextual-moss.svg";

/// Monochrome mask: the view supplies Moss ink and an Algae circle from the design tokens.
/// The irregular pieces follow the supplied raster mark; no screenshot background is bundled.
const MOSS_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256">
<path d="M114 34C114 28 135 27 143 31C148 34 139 49 134 58C130 66 119 64 116 55C114 48 113 40 114 34Z"/>
<path d="M91 38C104 31 109 38 110 53C112 63 109 69 102 70C118 64 129 66 128 72C117 79 111 91 105 95C99 97 109 80 101 77C91 80 82 72 81 61C78 51 82 42 91 38Z"/>
<path d="M143 53C150 42 154 32 165 35C174 36 184 42 187 47C189 52 172 54 159 58C147 63 132 64 143 53Z"/>
<path d="M138 69C149 63 169 59 179 64C190 70 182 85 181 96C180 106 178 115 175 106C171 85 154 77 141 75C133 74 132 72 138 69Z"/>
<path d="M185 56C193 47 202 57 198 66C194 75 182 72 182 65C182 61 183 58 185 56Z"/>
<path d="M67 58C74 51 81 68 84 79C88 90 86 109 86 120C86 130 71 129 59 122C47 116 45 105 46 89C47 74 59 62 67 58Z"/>
<path d="M198 74C208 62 218 79 215 90C213 100 207 113 198 114C186 114 185 109 187 98C189 88 193 80 198 74Z"/>
<path d="M214 104C221 91 227 106 227 122C228 135 226 146 220 146C207 147 207 133 208 121C209 113 210 110 214 104Z"/>
<path d="M38 104C44 97 51 125 53 143C54 154 56 169 50 172C41 173 35 148 34 132C33 119 34 108 38 104Z"/>
<path d="M62 132C69 130 81 129 84 138C87 151 88 172 80 179C70 191 60 174 58 158C56 145 56 137 62 132Z"/>
<path d="M125 93C136 94 136 110 147 123C156 133 158 139 158 150L157 170C157 181 149 182 138 177C120 169 109 150 108 138C107 121 109 103 118 96C120 94 123 93 125 93Z"/>
<path d="M136 94C147 91 154 96 156 108C158 118 159 124 153 118C146 111 142 104 136 94Z"/>
<path d="M184 116C197 108 207 118 207 131C206 144 198 151 195 167C193 178 188 180 178 177L178 127C178 121 179 118 184 116Z"/>
<path d="M210 149C220 141 224 151 221 160C218 172 210 185 203 182C195 181 200 168 203 159C205 154 207 151 210 149Z"/>
<path d="M57 176C60 172 69 184 71 190C73 198 63 197 59 191C55 184 54 179 57 176Z"/>
<path d="M89 183C100 179 110 185 109 163C109 155 116 161 122 169C131 180 127 181 117 181C109 181 118 193 111 209C108 221 99 216 86 211C74 207 69 201 75 193C79 187 82 185 89 183Z"/>
<path d="M125 186C136 179 142 183 138 196C136 208 130 220 120 221C111 221 118 210 119 201C120 194 119 190 125 186Z"/>
<path d="M151 186C157 180 176 197 176 208C177 216 154 221 143 220C135 220 143 204 145 197C147 191 148 187 151 186Z"/>
<path d="M171 180C179 179 191 180 199 183C205 187 195 201 189 201C178 200 171 191 164 185C159 181 165 180 171 180Z"/>
</svg>"#;

/// The default component icons plus [`ExtraIcons`] and the Moss mark.
pub struct AppAssets;

impl gpui_kit::AssetSource for AppAssets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<Cow<'static, [u8]>>> {
        if path == MOSS_ICON_PATH {
            return Ok(Some(Cow::Borrowed(MOSS_ICON)));
        }
        match gpui_kit::assets::Assets.load(path) {
            Ok(Some(data)) => Ok(Some(data)),
            _ => ExtraIcons.load(path),
        }
    }

    fn list(&self, path: &str) -> gpui_kit::Result<Vec<gpui_kit::SharedString>> {
        let mut all = gpui_kit::assets::Assets.list(path).unwrap_or_default();
        all.extend(ExtraIcons.list(path).unwrap_or_default());
        if MOSS_ICON_PATH.starts_with(path) {
            all.push(MOSS_ICON_PATH.into());
        }
        Ok(all)
    }
}
