use std::env;

use gpui::SharedString;
use pier_core::connections::ConnectionStore;
use pier_core::paths;
use pier_core::services::git::GitClient;
use pier_core::services::local_exec;

#[derive(Clone)]
pub struct ShellSnapshot {
    pub core_version: SharedString,
    pub workspace_path: SharedString,
    pub git_branch: SharedString,
    pub git_detail: SharedString,
    pub connections_value: SharedString,
    pub connections_detail: SharedString,
    pub local_machine_value: SharedString,
    pub local_machine_detail: SharedString,
    pub path_value: SharedString,
    pub path_detail: SharedString,
}

impl ShellSnapshot {
    pub fn load() -> Self {
        let current_dir = env::current_dir().ok();
        let workspace_path = current_dir
            .as_ref()
            .map(path_string)
            .unwrap_or_else(|| "Unavailable".into());

        let (git_branch, git_detail) = match current_dir.as_ref() {
            Some(dir) => match GitClient::open(&dir.to_string_lossy()) {
                Ok(client) => {
                    let branch = client
                        .branch_info()
                        .map(|info| {
                            if info.tracking.is_empty() {
                                info.name
                            } else {
                                format!("{} -> {}", info.name, info.tracking)
                            }
                        })
                        .unwrap_or_else(|error| format!("Branch unavailable: {error}"));
                    let detail = client
                        .status()
                        .map(|changes| {
                            let staged = changes.iter().filter(|change| change.staged).count();
                            let unstaged = changes.len().saturating_sub(staged);
                            format!("{staged} staged, {unstaged} unstaged")
                        })
                        .unwrap_or_else(|error| format!("Status unavailable: {error}"));
                    (branch, detail)
                }
                Err(error) => ("Git unavailable".into(), error.to_string()),
            },
            None => (
                "Git unavailable".into(),
                "Current directory could not be resolved".into(),
            ),
        };

        let (connections_value, connections_detail) = match ConnectionStore::load_default() {
            Ok(store) => {
                let detail = paths::connections_file()
                    .map(path_string)
                    .unwrap_or_else(|| "Connection store path unavailable".into());
                (
                    format!("{} saved SSH targets", store.connections.len()),
                    detail,
                )
            }
            Err(error) => (
                "Connections unavailable".into(),
                format!("Failed to load connection store: {error}"),
            ),
        };

        let (local_machine_value, local_machine_detail) = match local_exec::system_metrics() {
            Ok(metrics) => {
                let cpu = if metrics.cpu_pct >= 0.0 {
                    format!("{:.1}% CPU", metrics.cpu_pct)
                } else {
                    "CPU unavailable".into()
                };
                let mem = if metrics.mem_total_mb > 0.0 {
                    format!(
                        "{:.0} / {:.0} MB RAM, {}",
                        metrics.mem_used_mb, metrics.mem_total_mb, metrics.uptime
                    )
                } else if metrics.uptime.is_empty() {
                    "Memory and uptime unavailable".into()
                } else {
                    metrics.uptime
                };
                (cpu, mem)
            }
            Err(error) => (
                "Local metrics unavailable".into(),
                format!("Failed to probe local machine: {error}"),
            ),
        };

        let config_dir = paths::config_dir()
            .map(path_string)
            .unwrap_or_else(|| "Unavailable".into());
        let data_dir = paths::data_dir()
            .map(path_string)
            .unwrap_or_else(|| "Unavailable".into());
        let cache_dir = paths::cache_dir()
            .map(path_string)
            .unwrap_or_else(|| "Unavailable".into());

        Self {
            core_version: format!("pier-core {}", pier_core::VERSION).into(),
            workspace_path: workspace_path.into(),
            git_branch: git_branch.into(),
            git_detail: git_detail.into(),
            connections_value: connections_value.into(),
            connections_detail: connections_detail.into(),
            local_machine_value: local_machine_value.into(),
            local_machine_detail: local_machine_detail.into(),
            path_value: config_dir.into(),
            path_detail: format!("data: {data_dir}\ncache: {cache_dir}").into(),
        }
    }
}

fn path_string(path: impl AsRef<std::path::Path>) -> String {
    path.as_ref().display().to_string()
}
