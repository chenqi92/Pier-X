// Bundled SVG icons and IBM Plex font faces for the GPUI shell.
//
// gpui-component renders `IconName`/`Icon` (and its own internal chrome) by
// asking the app's `AssetSource` for `icons/<name>.svg`. We register a single
// RustEmbed source that ships both the gpui-component default icon set and the
// extra lucide glyphs Pier-X needs for tool/service branding (git-branch,
// database, container, shield, …) that the default set doesn't include.
//
// SVGs are tinted by the element's `text_color` (they use `currentColor`), so
// the same file recolors per service via `svg().path(..).text_color(..)`.
//
// The same embed also carries the IBM Plex Sans/Mono .ttf faces; `load_fonts`
// hands their bytes to the text system so the theme's "IBM Plex Sans"/"IBM Plex
// Mono" family names resolve without a system install (the repo ships these as
// .woff2 only, which font-kit can't read).

use std::borrow::Cow;

use anyhow::anyhow;
use gpui::{App, AssetSource, Result, SharedString};

#[derive(rust_embed::RustEmbed)]
#[folder = "assets"]
#[include = "icons/**/*.svg"]
#[include = "fonts/**/*.ttf"]
pub struct Assets;

/// Register every embedded .ttf with the text system. Call once at startup,
/// before the first window paints.
pub fn load_fonts(cx: &App) -> Result<()> {
    let fonts: Vec<Cow<'static, [u8]>> = Assets::iter()
        .filter(|p| p.ends_with(".ttf"))
        .filter_map(|p| Assets::get(&p).map(|f| f.data))
        .collect();
    cx.text_system().add_fonts(fonts)
}

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path.is_empty() {
            return Ok(None);
        }
        Self::get(path)
            .map(|f| Some(f.data))
            .ok_or_else(|| anyhow!("could not find asset at path \"{}\"", path))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(Self::iter()
            .filter_map(|p| p.starts_with(path).then(|| p.into()))
            .collect())
    }
}
