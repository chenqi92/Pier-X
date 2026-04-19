pub mod actions;
pub mod db_session;
pub mod git_session;
pub mod keybindings;
pub mod layout;
pub mod route;
pub mod shell_location;
pub mod ssh_session;
pub mod state;
pub mod statusbar;
pub mod toolbar;

use std::rc::Rc;

use gpui::{App, Window};

pub use actions::{
    CloseActiveTab, NewTab, OpenSettings, ToggleLeftPanel, ToggleRightPanel, ToggleTheme,
};
pub use route::Route;
pub use shell_location::{RemoteTarget, ShellLocation};
pub use state::PierApp;

/// Vestigial signature kept around so [`crate::views::terminal::TerminalPanel`]
/// retains its existing constructor. The 3-pane shell mounts the terminal
/// directly in the center column and passes a no-op closure.
pub type ActivationHandler = Rc<dyn Fn(Route, &mut Window, &mut App)>;
