use std::env;

use gpui::{div, prelude::*, px, IntoElement, SharedString, Window};
use pier_core::services::git::{BranchInfo, CommitInfo, FileStatus, GitClient, GitFileChange};

use crate::components::{text, Card, SectionLabel, StatusKind, StatusPill};
use crate::theme::{
    radius::RADIUS_SM,
    spacing::{SP_1_5, SP_2, SP_3, SP_4},
    theme,
    typography::{SIZE_BODY, SIZE_MONO_SMALL, SIZE_SMALL, WEIGHT_MEDIUM},
};

#[derive(IntoElement)]
pub struct GitView {
    snapshot: GitSnapshot,
}

impl GitView {
    pub fn new() -> Self {
        Self {
            snapshot: GitSnapshot::probe(),
        }
    }
}

impl Default for GitView {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for GitView {
    fn render(self, _: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let t = theme(cx);
        let snap = self.snapshot;

        let header = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_3)
            .child(text::h2("Git"))
            .child(match &snap.repo_state {
                RepoState::Open { .. } => StatusPill::new("repo: open", StatusKind::Success),
                RepoState::NotARepo => StatusPill::new("no repo", StatusKind::Warning),
                RepoState::Error => StatusPill::new("error", StatusKind::Error),
            });

        match snap.repo_state {
            RepoState::Open {
                branch,
                changes,
                log,
                repo_path,
            } => div()
                .size_full()
                .flex()
                .flex_col()
                .gap(SP_4)
                .p(SP_4)
                .child(header)
                .child(branch_card(t, &branch, &repo_path))
                .child(changes_card(t, &changes))
                .child(log_card(t, &log)),
            RepoState::NotARepo => div()
                .size_full()
                .flex()
                .flex_col()
                .gap(SP_4)
                .p(SP_4)
                .child(header)
                .child(
                    Card::new()
                        .child(SectionLabel::new("Repository"))
                        .child(
                            text::body("Current working directory is not inside a Git repository.")
                                .secondary(),
                        )
                        .child(text::mono(snap.cwd)),
                ),
            RepoState::Error => div()
                .size_full()
                .flex()
                .flex_col()
                .gap(SP_4)
                .p(SP_4)
                .child(header)
                .child(
                    Card::new()
                        .child(SectionLabel::new("Error"))
                        .child(text::body(snap.last_error.unwrap_or_default()).secondary()),
                ),
        }
    }
}

fn branch_card(t: &crate::theme::Theme, branch: &BranchInfo, repo_path: &SharedString) -> Card {
    let tracking: SharedString = if branch.tracking.is_empty() {
        "no upstream".into()
    } else {
        branch.tracking.clone().into()
    };
    let pace: SharedString = format!("{} ahead · {} behind", branch.ahead, branch.behind).into();

    Card::new()
        .child(SectionLabel::new("Current branch"))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(SP_2)
                .child(
                    div()
                        .text_size(SIZE_BODY)
                        .font_weight(WEIGHT_MEDIUM)
                        .text_color(t.color.text_primary)
                        .child(SharedString::from(branch.name.clone())),
                )
                .child(
                    div()
                        .text_size(SIZE_SMALL)
                        .text_color(t.color.text_tertiary)
                        .child(format!("→ {tracking}")),
                ),
        )
        .child(
            div()
                .text_size(SIZE_SMALL)
                .text_color(t.color.text_tertiary)
                .child(pace),
        )
        .child(
            div()
                .text_size(SIZE_MONO_SMALL)
                .font_family(t.font_mono.clone())
                .text_color(t.color.text_tertiary)
                .child(format!("repo: {repo_path}")),
        )
}

fn changes_card(t: &crate::theme::Theme, changes: &[GitFileChange]) -> Card {
    let staged = changes.iter().filter(|c| c.staged).count();
    let unstaged = changes.len().saturating_sub(staged);

    let mut card = Card::new().child(SectionLabel::new("Working tree")).child(
        div()
            .flex()
            .flex_row()
            .gap(SP_3)
            .child(StatusPill::new(
                format!("{staged} staged"),
                if staged > 0 {
                    StatusKind::Info
                } else {
                    StatusKind::Success
                },
            ))
            .child(StatusPill::new(
                format!("{unstaged} unstaged"),
                if unstaged > 0 {
                    StatusKind::Warning
                } else {
                    StatusKind::Success
                },
            )),
    );

    if changes.is_empty() {
        card = card.child(text::body("Working tree clean.").secondary());
    } else {
        for change in changes.iter().take(20) {
            card = card.child(file_change_row(t, change));
        }
        if changes.len() > 20 {
            card = card.child(
                div()
                    .text_size(SIZE_SMALL)
                    .text_color(t.color.text_tertiary)
                    .child(format!("… +{} more", changes.len() - 20)),
            );
        }
    }
    card
}

fn file_change_row(t: &crate::theme::Theme, change: &GitFileChange) -> impl IntoElement {
    let (badge, badge_color) = file_status_badge(change.status.clone());
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_2)
        .h(px(20.0))
        .child(
            div()
                .w(px(14.0))
                .h(px(14.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(2.0))
                .bg(badge_color)
                .text_color(t.color.text_inverse)
                .text_size(SIZE_SMALL)
                .font_weight(WEIGHT_MEDIUM)
                .child(SharedString::from(badge.to_string())),
        )
        .child(
            div()
                .text_size(SIZE_SMALL)
                .font_family(t.font_mono.clone())
                .text_color(if change.staged {
                    t.color.text_primary
                } else {
                    t.color.text_secondary
                })
                .child(SharedString::from(change.path.clone())),
        )
        .child(
            div()
                .text_size(SIZE_SMALL)
                .text_color(t.color.text_tertiary)
                .child(if change.staged { "[staged]" } else { "[work]" }),
        )
}

fn file_status_badge(status: FileStatus) -> (&'static str, gpui::Rgba) {
    use gpui::rgb;
    match status {
        FileStatus::Modified => ("M", rgb(0xf0a83a)),
        FileStatus::Added => ("A", rgb(0x5fb865)),
        FileStatus::Deleted => ("D", rgb(0xfa6675)),
        FileStatus::Renamed => ("R", rgb(0x3574f0)),
        FileStatus::Copied => ("C", rgb(0x3574f0)),
        FileStatus::Conflicted => ("!", rgb(0xfa6675)),
        FileStatus::Untracked => ("?", rgb(0x868a91)),
    }
}

fn log_card(t: &crate::theme::Theme, log: &[CommitInfo]) -> Card {
    let mut card = Card::new().padding(SP_3).child(
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(SP_2)
            .child(SectionLabel::new("Recent commits"))
            .child(
                div()
                    .text_size(SIZE_SMALL)
                    .text_color(t.color.text_tertiary)
                    .child(format!("{} entries", log.len())),
            ),
    );
    if log.is_empty() {
        card = card.child(text::body("No commits to show.").secondary());
        return card;
    }
    for c in log.iter().take(15) {
        card = card.child(commit_row(t, c));
    }
    card
}

fn commit_row(t: &crate::theme::Theme, c: &CommitInfo) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(SP_2)
        .h(px(22.0))
        .px(SP_1_5)
        .rounded(RADIUS_SM)
        .child(
            div()
                .w(px(64.0))
                .text_size(SIZE_MONO_SMALL)
                .font_family(t.font_mono.clone())
                .text_color(t.color.text_tertiary)
                .child(SharedString::from(c.short_hash.clone())),
        )
        .child(
            div()
                .flex_1()
                .text_size(SIZE_SMALL)
                .text_color(t.color.text_primary)
                .child(SharedString::from(c.message.clone())),
        )
        .child(
            div()
                .text_size(SIZE_SMALL)
                .text_color(t.color.text_tertiary)
                .child(SharedString::from(c.author.clone())),
        )
        .child(
            div()
                .text_size(SIZE_SMALL)
                .text_color(t.color.text_tertiary)
                .child(SharedString::from(c.relative_date.clone())),
        )
}

// ─────────────────────────────────────────────────────────
// Snapshot probe
// ─────────────────────────────────────────────────────────

enum RepoState {
    Open {
        branch: BranchInfo,
        changes: Vec<GitFileChange>,
        log: Vec<CommitInfo>,
        repo_path: SharedString,
    },
    NotARepo,
    Error,
}

struct GitSnapshot {
    cwd: SharedString,
    repo_state: RepoState,
    last_error: Option<SharedString>,
}

impl GitSnapshot {
    fn probe() -> Self {
        let cwd_path = env::current_dir().ok();
        let cwd: SharedString = cwd_path
            .as_ref()
            .map(|p| p.display().to_string().into())
            .unwrap_or_else(|| SharedString::from("Unavailable"));

        let cwd_str = match cwd_path.as_ref() {
            Some(p) => p.to_string_lossy().to_string(),
            None => {
                return Self {
                    cwd,
                    repo_state: RepoState::Error,
                    last_error: Some("Working directory unavailable".into()),
                }
            }
        };

        let client = match GitClient::open(&cwd_str) {
            Ok(c) => c,
            Err(e) => {
                let msg = e.to_string();
                let lower = msg.to_lowercase();
                if lower.contains("not a git") || lower.contains("does not exist") {
                    return Self {
                        cwd,
                        repo_state: RepoState::NotARepo,
                        last_error: None,
                    };
                }
                return Self {
                    cwd,
                    repo_state: RepoState::Error,
                    last_error: Some(msg.into()),
                };
            }
        };

        let branch = client.branch_info().unwrap_or(BranchInfo {
            name: "HEAD".into(),
            tracking: String::new(),
            ahead: 0,
            behind: 0,
        });
        let changes = client.status().unwrap_or_default();
        let log = client.log(15).unwrap_or_default();
        let repo_path: SharedString = client.repo_path().display().to_string().into();

        Self {
            cwd,
            repo_state: RepoState::Open {
                branch,
                changes,
                log,
                repo_path,
            },
            last_error: None,
        }
    }
}
