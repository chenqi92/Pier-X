use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

static ASSETS: &[(&str, &[u8])] = &[
    (
        "icons/arrow-down.svg",
        include_bytes!("../assets/icons/arrow-down.svg"),
    ),
    (
        "icons/arrow-up.svg",
        include_bytes!("../assets/icons/arrow-up.svg"),
    ),
    (
        "icons/chart-pie.svg",
        include_bytes!("../assets/icons/chart-pie.svg"),
    ),
    (
        "icons/check.svg",
        include_bytes!("../assets/icons/check.svg"),
    ),
    (
        "icons/chevron-down.svg",
        include_bytes!("../assets/icons/chevron-down.svg"),
    ),
    (
        "icons/chevron-left.svg",
        include_bytes!("../assets/icons/chevron-left.svg"),
    ),
    (
        "icons/chevron-right.svg",
        include_bytes!("../assets/icons/chevron-right.svg"),
    ),
    (
        "icons/close.svg",
        include_bytes!("../assets/icons/close.svg"),
    ),
    (
        "icons/container.svg",
        include_bytes!("../assets/icons/container.svg"),
    ),
    (
        "icons/database.svg",
        include_bytes!("../assets/icons/database.svg"),
    ),
    (
        "icons/delete.svg",
        include_bytes!("../assets/icons/delete.svg"),
    ),
    (
        "icons/ellipsis.svg",
        include_bytes!("../assets/icons/ellipsis.svg"),
    ),
    (
        "icons/file-text.svg",
        include_bytes!("../assets/icons/file-text.svg"),
    ),
    ("icons/file.svg", include_bytes!("../assets/icons/file.svg")),
    (
        "icons/folder-plus.svg",
        include_bytes!("../assets/icons/folder-plus.svg"),
    ),
    (
        "icons/folder.svg",
        include_bytes!("../assets/icons/folder.svg"),
    ),
    (
        "icons/gallery-vertical-end.svg",
        include_bytes!("../assets/icons/gallery-vertical-end.svg"),
    ),
    (
        "icons/git-branch.svg",
        include_bytes!("../assets/icons/git-branch.svg"),
    ),
    (
        "icons/git-commit.svg",
        include_bytes!("../assets/icons/git-commit.svg"),
    ),
    (
        "icons/globe.svg",
        include_bytes!("../assets/icons/globe.svg"),
    ),
    (
        "icons/inbox.svg",
        include_bytes!("../assets/icons/inbox.svg"),
    ),
    (
        "icons/inspector.svg",
        include_bytes!("../assets/icons/inspector.svg"),
    ),
    (
        "icons/layout-dashboard.svg",
        include_bytes!("../assets/icons/layout-dashboard.svg"),
    ),
    (
        "icons/loader.svg",
        include_bytes!("../assets/icons/loader.svg"),
    ),
    ("icons/map.svg", include_bytes!("../assets/icons/map.svg")),
    ("icons/moon.svg", include_bytes!("../assets/icons/moon.svg")),
    (
        "icons/panel-left-close.svg",
        include_bytes!("../assets/icons/panel-left-close.svg"),
    ),
    (
        "icons/panel-left-open.svg",
        include_bytes!("../assets/icons/panel-left-open.svg"),
    ),
    (
        "icons/panel-right-close.svg",
        include_bytes!("../assets/icons/panel-right-close.svg"),
    ),
    (
        "icons/panel-right-open.svg",
        include_bytes!("../assets/icons/panel-right-open.svg"),
    ),
    ("icons/play.svg", include_bytes!("../assets/icons/play.svg")),
    ("icons/plus.svg", include_bytes!("../assets/icons/plus.svg")),
    (
        "icons/refresh-cw.svg",
        include_bytes!("../assets/icons/refresh-cw.svg"),
    ),
    (
        "icons/replace.svg",
        include_bytes!("../assets/icons/replace.svg"),
    ),
    (
        "icons/server.svg",
        include_bytes!("../assets/icons/server.svg"),
    ),
    (
        "icons/settings-2.svg",
        include_bytes!("../assets/icons/settings-2.svg"),
    ),
    (
        "icons/settings.svg",
        include_bytes!("../assets/icons/settings.svg"),
    ),
    (
        "icons/square-terminal.svg",
        include_bytes!("../assets/icons/square-terminal.svg"),
    ),
    (
        "icons/square.svg",
        include_bytes!("../assets/icons/square.svg"),
    ),
    ("icons/sun.svg", include_bytes!("../assets/icons/sun.svg")),
    (
        "icons/triangle-alert.svg",
        include_bytes!("../assets/icons/triangle-alert.svg"),
    ),
    // — Added for the Git graph/managers views —
    (
        "icons/search.svg",
        include_bytes!("../assets/icons/search.svg"),
    ),
    ("icons/user.svg", include_bytes!("../assets/icons/user.svg")),
    ("icons/copy.svg", include_bytes!("../assets/icons/copy.svg")),
    (
        "icons/calendar.svg",
        include_bytes!("../assets/icons/calendar.svg"),
    ),
    (
        "icons/external-link.svg",
        include_bytes!("../assets/icons/external-link.svg"),
    ),
    ("icons/undo.svg", include_bytes!("../assets/icons/undo.svg")),
    (
        "icons/circle-check.svg",
        include_bytes!("../assets/icons/circle-check.svg"),
    ),
    (
        "icons/folder-open.svg",
        include_bytes!("../assets/icons/folder-open.svg"),
    ),
    (
        "icons/ellipsis-vertical.svg",
        include_bytes!("../assets/icons/ellipsis-vertical.svg"),
    ),
    (
        "icons/github.svg",
        include_bytes!("../assets/icons/github.svg"),
    ),
    // — File-type icons for the file browser row —
    (
        "icons/book-open.svg",
        include_bytes!("../assets/icons/book-open.svg"),
    ),
    (
        "icons/palette.svg",
        include_bytes!("../assets/icons/palette.svg"),
    ),
];

#[derive(Clone, Copy, Default)]
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(ASSETS
            .iter()
            .find(|(asset_path, _)| *asset_path == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let prefix = path.trim_matches('/');
        let prefix = if prefix.is_empty() {
            String::new()
        } else {
            format!("{prefix}/")
        };

        Ok(ASSETS
            .iter()
            .filter_map(|(asset_path, _)| {
                if prefix.is_empty() || asset_path.starts_with(&prefix) {
                    Some((*asset_path).into())
                } else {
                    None
                }
            })
            .collect())
    }
}
