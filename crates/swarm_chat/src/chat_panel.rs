use std::path::PathBuf;

use futures::StreamExt;
use gpui::{
    div, Context, Entity, EventEmitter, Focusable, FocusHandle,
    IntoElement, Render, Task, Window, InteractiveElement, ParentElement, Styled,
};
use serde::{Deserialize, Serialize};
use ui::prelude::*;
use uuid::Uuid;

use crate::codex_client::{CodexClient, CodexConfig, CodexEvent};
use crate::message_input::MessageInput;
use crate::message_list::MessageList;

pub enum ChatPanelEvent {
    MessageSent(String),
    FilePickerRequested,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub id: Uuid,
    pub role: MessageRole,
    pub content: String,
    pub timestamp: i64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

pub struct ChatPanel {
    messages: Vec<Message>,
    message_list: Entity<MessageList>,
    message_input: Entity<MessageInput>,
    focus_handle: FocusHandle,
    repo_path: Option<PathBuf>,
    conversation_id: Option<Uuid>,
    is_streaming: bool,
    codex_client: Option<CodexClient>,
    codex_session_id: Option<String>,
    status_message: Option<String>,
    // Both tasks must be kept alive: the stream task runs codex, the receiver task processes events
    _codex_stream_task: Option<Task<anyhow::Result<()>>>,
    _codex_receiver_task: Option<Task<()>>,
}

impl ChatPanel {
    pub fn new(
        repo_path: Option<PathBuf>,
        session_id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();

        let codex_session_id = session_id.clone();
        let conversation_id = session_id
            .and_then(|s| Uuid::parse_str(&s).ok())
            .or_else(|| Some(Uuid::new_v4()));

        let messages = Vec::new();
        let message_list = cx.new(|cx| MessageList::new(messages.clone(), window, cx));

        let message_input = cx.new(|cx| MessageInput::new(window, cx));
        cx.subscribe(&message_input, Self::handle_input_event).detach();

        let codex_client = repo_path.as_ref().map(|path| {
            CodexClient::new(CodexConfig {
                cli_path: "codex".to_string(),
                repo_root: path.clone(),
                add_dirs: Vec::new(),
            })
        });

        Self {
            messages,
            message_list,
            message_input,
            focus_handle,
            repo_path,
            conversation_id,
            is_streaming: false,
            codex_client,
            codex_session_id,
            status_message: None,
            _codex_stream_task: None,
            _codex_receiver_task: None,
        }
    }

    fn handle_input_event(
        &mut self,
        _input: Entity<MessageInput>,
        event: &MessageInputEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            MessageInputEvent::Submit(content) => {
                self.send_message(content.clone(), cx);
            }
            MessageInputEvent::FilePickerRequested => {
                cx.emit(ChatPanelEvent::FilePickerRequested);
            }
        }
    }

    fn send_message(&mut self, content: String, cx: &mut Context<Self>) {
        if content.trim().is_empty() {
            return;
        }

        log::info!("send_message called with content: {:?}", content);

        let message = Message {
            id: Uuid::new_v4(),
            role: MessageRole::User,
            content: content.clone(),
            timestamp: chrono_timestamp(),
        };

        self.messages.push(message);
        self.update_message_list(cx);
        cx.emit(ChatPanelEvent::MessageSent(content.clone()));

        if let Some(ref client) = self.codex_client {
            log::info!("Sending message to Codex CLI");
            self.status_message = Some("Thinking...".to_string());
            let session_id = self.codex_session_id.clone();
            let (mut rx, stream_task) = client.send_message(
                content,
                session_id,
                cx.background_executor().clone(),
            );

            let receiver_task = cx.spawn(async move |this, cx| {
                while let Some(event) = rx.next().await {
                    log::debug!("Received Codex event: {:?}", event);
                    this.update(cx, |this, cx| {
                        this.handle_codex_event(event, cx);
                    }).ok();
                }
            });
            // Both tasks must be stored to keep them alive - dropping cancels them
            self._codex_stream_task = Some(stream_task);
            self._codex_receiver_task = Some(receiver_task);
        } else {
            log::warn!("No codex_client available - repo_path: {:?}", self.repo_path);
            self.status_message = Some("No repository configured. Run with --repo <path> to enable Codex.".to_string());
        }

        cx.notify();
    }

    fn handle_codex_event(&mut self, event: CodexEvent, cx: &mut Context<Self>) {
        match event {
            CodexEvent::SessionStarted { session_id } => {
                self.codex_session_id = Some(session_id);
            }
            CodexEvent::Token { delta } => {
                self.append_streaming_token(delta, cx);
            }
            CodexEvent::Status { phase: _, message } => {
                self.status_message = message;
                cx.notify();
            }
            CodexEvent::Completed { session_id, .. } => {
                if let Some(sid) = session_id {
                    self.codex_session_id = Some(sid);
                }
                self.finish_streaming(cx);
                self.status_message = None;
            }
            CodexEvent::Error { message } => {
                self.status_message = Some(format!("Error: {}", message));
                self.finish_streaming(cx);
            }
        }
    }

    fn update_message_list(&mut self, cx: &mut Context<Self>) {
        self.message_list.update(cx, |list, cx| {
            list.set_messages(self.messages.clone(), cx);
        });
    }

    pub fn append_streaming_token(&mut self, token: String, cx: &mut Context<Self>) {
        if !self.is_streaming {
            self.is_streaming = true;
            let message = Message {
                id: Uuid::new_v4(),
                role: MessageRole::Assistant,
                content: token,
                timestamp: chrono_timestamp(),
            };
            self.messages.push(message);
        } else if let Some(last) = self.messages.last_mut() {
            if last.role == MessageRole::Assistant {
                last.content.push_str(&token);
            }
        }
        self.update_message_list(cx);
        cx.notify();
    }

    pub fn finish_streaming(&mut self, cx: &mut Context<Self>) {
        self.is_streaming = false;
        cx.notify();
    }

    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    pub fn repo_path(&self) -> Option<&PathBuf> {
        self.repo_path.as_ref()
    }

    pub fn conversation_id(&self) -> Option<Uuid> {
        self.conversation_id
    }

    pub fn codex_session_id(&self) -> Option<&String> {
        self.codex_session_id.as_ref()
    }

    pub fn load_conversation(&mut self, conversation: &swarm_store::Conversation, cx: &mut Context<Self>) {
        self.conversation_id = Some(conversation.id);
        self.codex_session_id = conversation.codex_session_id.clone();
        self.messages = conversation.messages.iter().map(|m| Message {
            id: m.id,
            role: match m.role {
                swarm_store::MessageRole::User => MessageRole::User,
                swarm_store::MessageRole::Assistant => MessageRole::Assistant,
                swarm_store::MessageRole::System => MessageRole::System,
            },
            content: m.content.clone(),
            timestamp: m.timestamp,
        }).collect();
        self.is_streaming = false;
        self.status_message = None;
        self._codex_stream_task = None;
        self._codex_receiver_task = None;
        self.update_message_list(cx);
        cx.notify();
    }

    pub fn clear_conversation(&mut self, cx: &mut Context<Self>) {
        self.conversation_id = Some(Uuid::new_v4());
        self.codex_session_id = None;
        self.messages.clear();
        self.is_streaming = false;
        self.status_message = None;
        self._codex_stream_task = None;
        self._codex_receiver_task = None;
        self.update_message_list(cx);
        cx.notify();
    }

    pub fn to_store_conversation(&self) -> swarm_store::Conversation {
        let messages = self.messages.iter().map(|m| {
            swarm_store::Message {
                id: m.id,
                role: match m.role {
                    MessageRole::User => swarm_store::MessageRole::User,
                    MessageRole::Assistant => swarm_store::MessageRole::Assistant,
                    MessageRole::System => swarm_store::MessageRole::System,
                },
                content: m.content.clone(),
                timestamp: m.timestamp,
            }
        }).collect();

        let now = chrono_timestamp();
        let mut conv = swarm_store::Conversation {
            id: self.conversation_id.unwrap_or_else(Uuid::new_v4),
            title: None,
            codex_session_id: self.codex_session_id.clone(),
            messages,
            created_at: now,
            updated_at: now,
        };

        // Generate title from first user message
        if conv.title.is_none() {
            conv.title = Some(conv.generate_title());
        }

        conv
    }
}

fn chrono_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl EventEmitter<ChatPanelEvent> for ChatPanel {}

impl Focusable for ChatPanel {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ChatPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let status_message = self.status_message.clone();

        div()
            .key_context("ChatPanel")
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.colors().background)
            .child(
                div()
                    .flex_1()
                    .overflow_hidden()
                    .child(self.message_list.clone()),
            )
            .when_some(status_message.clone(), |this, msg| {
                this.child(
                    gpui::div()
                        .px_4()
                        .py_2()
                        .text_sm()
                        .text_color(theme.colors().text_muted)
                        .child(msg)
                )
            })
            .child(
                div()
                    .flex_shrink_0()
                    .border_t_1()
                    .border_color(theme.colors().border)
                    .p_2()
                    .child(self.message_input.clone()),
            )
    }
}

pub use crate::message_input::MessageInputEvent;
