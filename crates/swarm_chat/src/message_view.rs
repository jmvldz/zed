use gpui::{div, IntoElement, ParentElement, RenderOnce, Styled, Window, App};
use ui::prelude::*;

use crate::chat_panel::{Message, MessageRole};

#[derive(IntoElement)]
pub struct MessageView {
    message: Message,
}

impl MessageView {
    pub fn new(message: Message) -> Self {
        Self { message }
    }
}

impl RenderOnce for MessageView {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let is_user = self.message.role == MessageRole::User;

        let bg_color = if is_user {
            theme.colors().element_background
        } else {
            theme.colors().editor_background
        };

        let role_label = match self.message.role {
            MessageRole::User => "You",
            MessageRole::Assistant => "Assistant",
            MessageRole::System => "System",
        };

        div()
            .w_full()
            .flex()
            .flex_col()
            .when(is_user, |div| div.items_end())
            .when(!is_user, |div| div.items_start())
            .child(
                div()
                    .max_w(px(600.))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.colors().text_muted)
                            .child(role_label)
                    )
                    .child(
                        div()
                            .px_4()
                            .py_3()
                            .rounded_lg()
                            .bg(bg_color)
                            .child(self.message.content.clone())
                    )
            )
    }
}
