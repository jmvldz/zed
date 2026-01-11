use editor::{Editor, EditorEvent};
use gpui::{
    actions, div, App, Context, Entity, EventEmitter, Focusable, FocusHandle,
    IntoElement, Render, Window, InteractiveElement, ParentElement, Styled,
};
use language::language_settings::SoftWrap;
use ui::prelude::*;

actions!(swarm_chat, [SendMessage, OpenFilePicker]);

pub enum MessageInputEvent {
    Submit(String),
    FilePickerRequested,
}

pub struct MessageInput {
    editor: Entity<Editor>,
}

impl MessageInput {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let editor = cx.new(|cx| {
            let mut editor = Editor::auto_height(1, 10, window, cx);
            editor.set_placeholder_text("Type a message...", window, cx);
            editor.set_soft_wrap_mode(SoftWrap::EditorWidth, cx);
            editor
        });

        cx.subscribe_in(&editor, window, Self::handle_editor_event)
            .detach();

        Self { editor }
    }

    fn handle_editor_event(
        &mut self,
        _editor: &Entity<Editor>,
        event: &EditorEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let EditorEvent::BufferEdited { .. } = event {
            cx.notify();
        }
    }

    fn submit(&mut self, _: &SendMessage, window: &mut Window, cx: &mut Context<Self>) {
        let content = self.editor.read(cx).text(cx);
        log::info!("Submit called with content: {:?}", content);
        if !content.trim().is_empty() {
            cx.emit(MessageInputEvent::Submit(content));
            self.editor.update(cx, |editor, cx| {
                editor.clear(window, cx);
            });
            cx.notify();
        }
    }

    fn open_file_picker(&mut self, _: &OpenFilePicker, _window: &mut Window, cx: &mut Context<Self>) {
        cx.emit(MessageInputEvent::FilePickerRequested);
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.editor.focus_handle(cx).focus(window, cx);
    }

    pub fn content(&self, cx: &App) -> String {
        self.editor.read(cx).text(cx)
    }

    pub fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor.update(cx, |editor, cx| {
            editor.clear(window, cx);
        });
        cx.notify();
    }
}

impl EventEmitter<MessageInputEvent> for MessageInput {}

impl Focusable for MessageInput {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.editor.focus_handle(cx)
    }
}

impl Render for MessageInput {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        div()
            .w_full()
            .flex()
            .flex_row()
            .gap_2()
            .child(
                div()
                    .id("editor-container")
                    .key_context("MessageInput")
                    .on_action(cx.listener(Self::submit))
                    .on_action(cx.listener(Self::open_file_picker))
                    .flex_1()
                    .px_3()
                    .py_2()
                    .rounded_lg()
                    .bg(theme.colors().editor_background)
                    .border_1()
                    .border_color(theme.colors().border)
                    .min_h(px(36.))
                    .child(self.editor.clone()),
            )
            .child(
                ui::Button::new("send-button", "Send")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.submit(&SendMessage, window, cx);
                    }))
            )
    }
}
