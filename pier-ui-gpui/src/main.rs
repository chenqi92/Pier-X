use std::env;

use gpui::{
    div, prelude::*, px, rgb, size, App, Application, Bounds, Context, SharedString, Window,
    WindowBounds, WindowOptions,
};
use pier_core::connections::ConnectionStore;
use pier_core::paths;
use pier_core::services::git::GitClient;
use pier_core::services::local_exec;

#[derive(Clone)]
struct ShellSnapshot {
    core_version: SharedString,
    workspace_path: SharedString,
    repo_root: SharedString,
    git_branch: SharedString,
    git_detail: SharedString,
    connections_value: SharedString,
    connections_detail: SharedString,
    local_machine_value: SharedString,
    local_machine_detail: SharedString,
    path_value: SharedString,
    path_detail: SharedString,
}

impl ShellSnapshot {
    fn load() -> Self {
        let current_dir = env::current_dir().ok();
        let workspace_path = current_dir
            .as_ref()
            .map(path_string)
            .unwrap_or_else(|| "Unavailable".into());

        let (repo_root, git_branch, git_detail) = match current_dir.as_ref() {
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
                    (path_string(client.repo_path()), branch, detail)
                }
                Err(error) => (
                    "Not inside a Git repository".into(),
                    "Git unavailable".into(),
                    error.to_string(),
                ),
            },
            None => (
                "Working directory unavailable".into(),
                "Git unavailable".into(),
                "Current directory could not be resolved".into(),
            ),
        };

        let (connections_value, connections_detail) = match ConnectionStore::load_default() {
            Ok(store) => {
                let detail = paths::connections_file()
                    .map(path_string)
                    .unwrap_or_else(|| "Connection store path unavailable".into());
                (format!("{} saved SSH targets", store.connections.len()), detail)
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
            repo_root: repo_root.into(),
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

struct PierWorkbench {
    snapshot: ShellSnapshot,
}

impl PierWorkbench {
    fn new() -> Self {
        Self {
            snapshot: ShellSnapshot::load(),
        }
    }
}

impl Render for PierWorkbench {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(0x0b1220))
            .text_color(rgb(0xf4f7fb))
            .p_6()
            .gap_4()
            .flex()
            .flex_col()
            .child(
                div()
                    .gap_2()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(gpui::FontWeight::BOLD)
                            .child("Pier-X GPUI Reset"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0x90a3bf))
                            .child("Native Rust shell scaffold with direct pier-core integration."),
                    ),
            )
            .child(metric_card(
                "Core",
                &self.snapshot.core_version,
                &self.snapshot.workspace_path,
            ))
            .child(metric_card(
                "Git Workspace",
                &self.snapshot.git_branch,
                &format_shared("repo: ", &self.snapshot.repo_root, "\n", &self.snapshot.git_detail),
            ))
            .child(metric_card(
                "Connections",
                &self.snapshot.connections_value,
                &self.snapshot.connections_detail,
            ))
            .child(metric_card(
                "Local Machine",
                &self.snapshot.local_machine_value,
                &self.snapshot.local_machine_detail,
            ))
            .child(metric_card(
                "App Paths",
                &self.snapshot.path_value,
                &self.snapshot.path_detail,
            ))
            .child(
                div()
                    .gap_1()
                    .flex()
                    .flex_col()
                    .p_4()
                    .bg(rgb(0x10192c))
                    .border_1()
                    .border_color(rgb(0x22314c))
                    .rounded(px(14.0))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0x9fb2d1))
                            .child("Next slices"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .child("1. Replace this dashboard with a dock/workbench layout."),
                    )
                    .child(
                        div()
                            .text_sm()
                            .child("2. Wire terminal sessions directly from pier-core without IPC."),
                    )
                    .child(
                        div()
                            .text_sm()
                            .child("3. Migrate Git, SSH, and data panels as native GPUI views."),
                    ),
            )
    }
}

fn metric_card(title: &'static str, value: &SharedString, detail: &SharedString) -> impl IntoElement {
    div()
        .gap_1()
        .flex()
        .flex_col()
        .p_4()
        .bg(rgb(0x10192c))
        .border_1()
        .border_color(rgb(0x22314c))
        .rounded(px(14.0))
        .child(
            div()
                .text_sm()
                .text_color(rgb(0x9fb2d1))
                .child(title),
        )
        .child(div().text_xl().child(value.clone()))
        .child(
            div()
                .text_sm()
                .text_color(rgb(0xb8c6db))
                .child(detail.clone()),
        )
}

fn format_shared(
    prefix: &str,
    value: &SharedString,
    separator: &str,
    suffix: &SharedString,
) -> SharedString {
    format!("{prefix}{value}{separator}{suffix}").into()
}

fn path_string(path: impl AsRef<std::path::Path>) -> String {
    path.as_ref().display().to_string()
}

fn main() {
    let app = Application::new();

    app.run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1100.0), px(760.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |_window, cx| cx.new(|_| PierWorkbench::new()),
        )
        .expect("failed to open GPUI window");
    });
}
