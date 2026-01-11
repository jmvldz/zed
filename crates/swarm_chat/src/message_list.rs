use gpui::{
    div, Context, Focusable, FocusHandle, IntoElement, Render, Window,
    InteractiveElement, ParentElement, Styled,
};
use ui::prelude::*;

use crate::chat_panel::Message;
use crate::message_view::MessageView;

pub struct MessageList {
    messages: Vec<Message>,
    focus_handle: FocusHandle,
}

impl MessageList {
    pub fn new(messages: Vec<Message>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();

        Self {
            messages,
            focus_handle,
        }
    }

    pub fn set_messages(&mut self, messages: Vec<Message>, cx: &mut Context<Self>) {
        self.messages = messages;
        cx.notify();
    }
}

impl Focusable for MessageList {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for MessageList {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        v_flex()
            .id("message-list")
            .key_context("MessageList")
            .track_focus(&self.focus_handle)
            .size_full()
            .overflow_y_scroll()
            .gap_2()
            .p_4()
            .bg(theme.colors().background)
            .children(
                self.messages
                    .iter()
                    .map(|message| MessageView::new(message.clone()))
            )
            .when(self.messages.is_empty(), |this| {
                this.child(
                    div()
                        .flex()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .text_color(theme.colors().text_muted)
                        .child("Start a conversation...")
                )
            })
    }
}
