pub mod actions;
pub mod layout;
pub mod route;
pub mod state;

use std::rc::Rc;

use gpui::{App, Window};

pub use actions::ToggleTheme;
pub use route::Route;
pub use state::PierApp;

/// Vestigial signature kept around so [`crate::views::terminal::TerminalPanel`]
/// retains its existing constructor. The 3-pane shell mounts the terminal
/// directly in the center column and passes a no-op closure.
pub type ActivationHandler = Rc<dyn Fn(Route, &mut Window, &mut App)>;
