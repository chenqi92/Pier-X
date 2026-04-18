use std::env;

use gpui::SharedString;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellSnapshot {
    pub workspace_path: SharedString,
}

impl ShellSnapshot {
    pub fn load() -> Self {
        let workspace_path = env::current_dir()
            .ok()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "Unavailable".to_string());

        Self {
            workspace_path: workspace_path.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ShellSnapshot;

    #[test]
    fn load_provides_workspace_path() {
        let snapshot = ShellSnapshot::load();
        assert!(!snapshot.workspace_path.is_empty());
    }
}
