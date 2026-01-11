mod conversation;
mod db_schema;

pub use conversation::{Conversation, ConversationStore, Message, MessageRole};

use gpui::App;

pub fn init(cx: &mut App) {
    db_schema::init(cx);
}
