use std::path::PathBuf;
use std::process::Command;

use gpui::{
    div, Context, Entity, Focusable, FocusHandle, IntoElement, Render, Window,
    InteractiveElement, ParentElement, Styled,
};
use swarm_chat::{ChatPanel, ChatSidebar, ChatSidebarEvent};
use ui::prelude::*;

#[derive(Clone, Debug, Default)]
pub struct GitStatus {
    pub branch: Option<String>,
    pub has_changes: bool,
}

pub struct SwarmWindow {
    chat_panel: Entity<ChatPanel>,
    chat_sidebar: Entity<ChatSidebar>,
    focus_handle: FocusHandle,
    repo_path: Option<PathBuf>,
    git_status: GitStatus,
}

impl SwarmWindow {
    pub fn new(
        repo_path: Option<PathBuf>,
        session_id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        let chat_panel = cx.new(|cx| ChatPanel::new(repo_path.clone(), session_id, window, cx));
        let chat_sidebar = cx.new(|cx| ChatSidebar::new(cx));

        // Subscribe to sidebar events
        cx.subscribe(&chat_sidebar, Self::handle_sidebar_event).detach();

        // Subscribe to chat panel events to save conversations
        cx.subscribe(&chat_panel, Self::handle_chat_event).detach();

        // Set the active conversation in sidebar if we have one
        let conversation_id = chat_panel.read(cx).conversation_id();
        chat_sidebar.update(cx, |sidebar, _cx| {
            sidebar.set_active_conversation(conversation_id);
        });

        let git_status = repo_path.as_ref()
            .map(|path| Self::fetch_git_status(path))
            .unwrap_or_default();

        Self {
            chat_panel,
            chat_sidebar,
            focus_handle,
            repo_path,
            git_status,
        }
    }

    fn handle_sidebar_event(
        &mut self,
        _sidebar: Entity<ChatSidebar>,
        event: &ChatSidebarEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            ChatSidebarEvent::NewConversation => {
                self.chat_panel.update(cx, |panel, cx| {
                    panel.clear_conversation(cx);
                });
                let conv_id = self.chat_panel.read(cx).conversation_id();
                self.chat_sidebar.update(cx, |sidebar, _cx| {
                    sidebar.set_active_conversation(conv_id);
                });
                cx.notify();
            }
            ChatSidebarEvent::ConversationSelected(id) => {
                let conversation = self.chat_sidebar.read(cx).store().get(id).cloned();
                if let Some(conv) = conversation {
                    self.chat_panel.update(cx, |panel, cx| {
                        panel.load_conversation(&conv, cx);
                    });
                    self.chat_sidebar.update(cx, |sidebar, _cx| {
                        sidebar.set_active_conversation(Some(*id));
                    });
                }
                cx.notify();
            }
            ChatSidebarEvent::ConversationDeleted(id) => {
                // If the deleted conversation was active, clear the panel
                let active_id = self.chat_panel.read(cx).conversation_id();
                if active_id == Some(*id) {
                    self.chat_panel.update(cx, |panel, cx| {
                        panel.clear_conversation(cx);
                    });
                }
                cx.notify();
            }
        }
    }

    fn handle_chat_event(
        &mut self,
        _panel: Entity<ChatPanel>,
        event: &swarm_chat::ChatPanelEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            swarm_chat::ChatPanelEvent::MessageSent(_) => {
                // Save the current conversation after a message is sent
                self.save_current_conversation(cx);
            }
            swarm_chat::ChatPanelEvent::FilePickerRequested => {
                // Handle file picker if needed
            }
        }
    }

    fn save_current_conversation(&mut self, cx: &mut Context<Self>) {
        let conversation = self.chat_panel.read(cx).to_store_conversation();
        let conv_id = conversation.id;

        self.chat_sidebar.update(cx, |sidebar, cx| {
            // Update or add the conversation
            if sidebar.store_mut().get_mut(&conv_id).is_some() {
                // Update existing conversation
                if let Some(existing) = sidebar.store_mut().get_mut(&conv_id) {
                    existing.messages = conversation.messages.clone();
                    existing.codex_session_id = conversation.codex_session_id.clone();
                    existing.updated_at = conversation.updated_at;
                    if existing.title.is_none() {
                        existing.title = conversation.title.clone();
                    }
                }
            } else {
                // Add new conversation
                sidebar.add_conversation(conversation, cx);
            }
            sidebar.set_active_conversation(Some(conv_id));
            sidebar.save(cx);
        });
        cx.notify();
    }

    fn fetch_git_status(repo_path: &PathBuf) -> GitStatus {
        let branch = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(repo_path)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());

        let has_changes = Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(repo_path)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| !o.stdout.is_empty())
            .unwrap_or(false);

        GitStatus { branch, has_changes }
    }

    fn repo_name(&self) -> Option<String> {
        self.repo_path.as_ref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .map(|s| s.to_string())
    }
}

impl Focusable for SwarmWindow {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SwarmWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let repo_name = self.repo_name();
        let git_status = self.git_status.clone();

        div()
            .key_context("SwarmWindow")
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.colors().background)
            .text_color(theme.colors().text)
            .font_family(".SystemFont")
            .child(
                div()
                    .flex_shrink_0()
                    .h(px(36.))
                    .pl(px(78.))  // Leave space for macOS traffic light buttons
                    .pr_4()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .border_b_1()
                    .border_color(theme.colors().border)
                    .bg(theme.colors().title_bar_background)
                    .when_some(repo_name, |this, name| {
                        this.child(
                            div()
                                .px_2()
                                .py(px(2.))
                                .rounded_md()
                                .bg(theme.colors().element_background)
                                .text_sm()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child(name)
                        )
                    })
                    .when_some(git_status.branch, |this, branch| {
                        this.child(
                            div()
                                .flex()
                                .flex_row()
                                .gap_1()
                                .items_center()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.colors().text_muted)
                                        .child(branch)
                                )
                                .when(git_status.has_changes, |this| {
                                    this.child(
                                        div()
                                            .size(px(6.))
                                            .rounded_full()
                                            .bg(theme.status().modified)
                                    )
                                })
                        )
                    })
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_row()
                    .overflow_hidden()
                    .child(self.chat_sidebar.clone())
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .child(self.chat_panel.clone())
                    )
            )
    }
}
