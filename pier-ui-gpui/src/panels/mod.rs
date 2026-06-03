// Right-side tool panels.
//
// Each tool that isn't Git/Monitor (those live in shell.rs with shell-level
// state) has a self-contained gpui View in its own file here. `PanelViews`
// owns one entity per panel and `for_svc` maps the active service to the entity
// to display. The shell delegates to this; see Shell::right_panel.
//
// To flesh out a panel, edit ONLY its own file (e.g. panels/docker.rs) — the
// module list and dispatch below are already wired, so parallel work on
// different panels never touches a shared file.

pub mod db;
pub mod docker;
pub mod firewall;
pub mod logs;
pub mod markdown;
pub mod search;
pub mod sftp;
pub mod software;
pub mod webserver;

use gpui::{AnyElement, AppContext, Context, Entity, IntoElement};

use crate::shell::{Shell, Svc};

/// One entity per non-Git/Monitor tool panel.
pub struct PanelViews {
    docker: Entity<docker::DockerPanel>,
    db: Entity<db::DbPanel>,
    sftp: Entity<sftp::SftpPanel>,
    logs: Entity<logs::LogsPanel>,
    markdown: Entity<markdown::MarkdownPanel>,
    firewall: Entity<firewall::FirewallPanel>,
    search: Entity<search::SearchPanel>,
    webserver: Entity<webserver::WebserverPanel>,
    software: Entity<software::SoftwarePanel>,
}

impl PanelViews {
    pub fn new(cx: &mut Context<Shell>) -> Self {
        Self {
            docker: cx.new(docker::DockerPanel::new),
            db: cx.new(db::DbPanel::new),
            sftp: cx.new(sftp::SftpPanel::new),
            logs: cx.new(logs::LogsPanel::new),
            markdown: cx.new(markdown::MarkdownPanel::new),
            firewall: cx.new(firewall::FirewallPanel::new),
            search: cx.new(search::SearchPanel::new),
            webserver: cx.new(webserver::WebserverPanel::new),
            software: cx.new(software::SoftwarePanel::new),
        }
    }

    /// The panel element for `svc`, or `None` for Git/Monitor (handled by the
    /// shell). All four relational/KV DB tools share the one DB panel.
    pub fn for_svc(&self, svc: Svc) -> Option<AnyElement> {
        let el = match svc {
            Svc::Docker => self.docker.clone().into_any_element(),
            Svc::Mysql | Svc::Postgres | Svc::Redis | Svc::Sqlite => {
                self.db.clone().into_any_element()
            }
            Svc::Sftp => self.sftp.clone().into_any_element(),
            Svc::Log => self.logs.clone().into_any_element(),
            Svc::Markdown => self.markdown.clone().into_any_element(),
            Svc::Firewall => self.firewall.clone().into_any_element(),
            Svc::Search => self.search.clone().into_any_element(),
            Svc::Webserver => self.webserver.clone().into_any_element(),
            Svc::Software => self.software.clone().into_any_element(),
            Svc::Git | Svc::Monitor => return None,
        };
        Some(el)
    }
}
