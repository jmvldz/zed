mod chat_panel;
mod chat_sidebar;
mod codex_client;
pub mod message_input;
mod message_list;
mod message_view;

pub use chat_panel::{ChatPanel, ChatPanelEvent};
pub use chat_sidebar::{ChatSidebar, ChatSidebarEvent};
pub use codex_client::{CodexClient, CodexConfig, CodexEvent};

use gpui::App;

pub fn init(_cx: &mut App) {
    // Register actions and initialize chat system
}
