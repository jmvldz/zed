use gpui::{
    div, px, Context, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Window,
};
use swarm_store::{Conversation, ConversationStore};
use ui::prelude::*;
use uuid::Uuid;

pub enum ChatSidebarEvent {
    NewConversation,
    ConversationSelected(Uuid),
    ConversationDeleted(Uuid),
}

pub struct ChatSidebar {
    store: ConversationStore,
    active_conversation_id: Option<Uuid>,
    focus_handle: FocusHandle,
}

impl ChatSidebar {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let store = ConversationStore::load().unwrap_or_default();

        Self {
            store,
            active_conversation_id: None,
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn set_active_conversation(&mut self, id: Option<Uuid>) {
        self.active_conversation_id = id;
    }

    pub fn active_conversation_id(&self) -> Option<Uuid> {
        self.active_conversation_id
    }

    pub fn store(&self) -> &ConversationStore {
        &self.store
    }

    pub fn store_mut(&mut self) -> &mut ConversationStore {
        &mut self.store
    }

    pub fn add_conversation(&mut self, conversation: Conversation, cx: &mut Context<Self>) {
        self.store.add(conversation);
        self.save(cx);
    }

    pub fn save(&self, _cx: &mut Context<Self>) {
        if let Err(e) = self.store.save() {
            log::error!("Failed to save conversations: {}", e);
        }
    }

    pub fn reload(&mut self, _cx: &mut Context<Self>) {
        match ConversationStore::load() {
            Ok(store) => self.store = store,
            Err(e) => log::error!("Failed to reload conversations: {}", e),
        }
    }

    fn new_conversation(&mut self, cx: &mut Context<Self>) {
        cx.emit(ChatSidebarEvent::NewConversation);
    }

    fn select_conversation(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.active_conversation_id = Some(id);
        cx.emit(ChatSidebarEvent::ConversationSelected(id));
        cx.notify();
    }

    fn delete_conversation(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.store.remove(&id);
        self.save(cx);
        cx.emit(ChatSidebarEvent::ConversationDeleted(id));
        cx.notify();
    }

    fn format_timestamp(timestamp: i64) -> String {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let diff = now - timestamp;

        if diff < 60 {
            "Just now".to_string()
        } else if diff < 3600 {
            format!("{}m ago", diff / 60)
        } else if diff < 86400 {
            format!("{}h ago", diff / 3600)
        } else if diff < 604800 {
            format!("{}d ago", diff / 86400)
        } else {
            format!("{}w ago", diff / 604800)
        }
    }
}

impl EventEmitter<ChatSidebarEvent> for ChatSidebar {}

impl Focusable for ChatSidebar {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ChatSidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let conversations: Vec<Conversation> = self.store.list_recent(50)
            .into_iter()
            .cloned()
            .collect();
        let active_id = self.active_conversation_id;

        div()
            .id("chat-sidebar")
            .w(px(220.))
            .h_full()
            .flex()
            .flex_col()
            .bg(theme.colors().surface_background)
            .border_r_1()
            .border_color(theme.colors().border)
            // Header with "+ New" button
            .child(
                div()
                    .p_2()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(theme.colors().text_muted)
                            .child("CHATS")
                    )
                    .child(
                        div()
                            .id("new-chat-button")
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .cursor_pointer()
                            .text_sm()
                            .text_color(theme.colors().text)
                            .bg(theme.colors().element_background)
                            .hover(|style| style.bg(theme.colors().element_hover))
                            .on_click(cx.listener(|this, _, _window, cx| {
                                this.new_conversation(cx);
                            }))
                            .child("+ New")
                    )
            )
            // Conversation list
            .child(
                v_flex()
                    .id("conversation-list")
                    .flex_1()
                    .overflow_y_scroll()
                    .p_1()
                    .children(conversations.into_iter().map(|conv| {
                        let conv_id = conv.id;
                        let is_active = active_id == Some(conv_id);
                        let title = conv.title.clone()
                            .unwrap_or_else(|| conv.generate_title());
                        let timestamp = Self::format_timestamp(conv.updated_at);

                        div()
                            .id(SharedString::from(format!("conv-{}", conv_id)))
                            .w_full()
                            .px_2()
                            .py_1()
                            .my(px(1.))
                            .rounded_md()
                            .cursor_pointer()
                            .group("conversation-item")
                            .when(is_active, |this| {
                                this.bg(theme.colors().element_selected)
                            })
                            .when(!is_active, |this| {
                                this.hover(|style| style.bg(theme.colors().element_hover))
                            })
                            .on_click(cx.listener(move |this, _, _window, cx| {
                                this.select_conversation(conv_id, cx);
                            }))
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .flex_1()
                                            .overflow_hidden()
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_ellipsis()
                                                    .overflow_hidden()
                                                    .whitespace_nowrap()
                                                    .child(title)
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(theme.colors().text_muted)
                                                    .child(timestamp)
                                            )
                                    )
                                    // Delete button (visible on hover)
                                    .child(
                                        div()
                                            .id(SharedString::from(format!("delete-{}", conv_id)))
                                            .ml_1()
                                            .px_1()
                                            .rounded_sm()
                                            .cursor_pointer()
                                            .text_xs()
                                            .text_color(theme.colors().text_muted)
                                            .invisible()
                                            .group_hover("conversation-item", |style| style.visible())
                                            .hover(|style| style
                                                .text_color(theme.status().deleted)
                                                .bg(theme.colors().element_hover)
                                            )
                                            .on_click(cx.listener(move |this, _, _window, cx| {
                                                this.delete_conversation(conv_id, cx);
                                            }))
                                            .child("×")
                                    )
                            )
                    }))
            )
    }
}
