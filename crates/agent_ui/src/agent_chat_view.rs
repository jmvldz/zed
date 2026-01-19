use std::any::Any;
use std::sync::Arc;

use anyhow::Result;
use gpui::{
    App, AsyncWindowContext, Context, Entity, EventEmitter, FocusHandle, Focusable,
    SharedString, Subscription, Task, WeakEntity, Window, prelude::*,
};
use project::{Project, ProjectPath};
use prompt_store::PromptBuilder;
use ui::{prelude::*, Color, Icon, IconName, Label};
use workspace::{
    Item, ItemId, ItemNavHistory, SerializableItem, Workspace, WorkspaceId,
    item::{BreadcrumbText, ItemBufferKind, ItemEvent, TabContentParams},
    searchable::SearchableItemHandle,
};

use crate::agent_chat_content::{AgentChatContent, AgentChatContentEvent};

use zed_actions;

pub enum AgentChatEvent {
    TitleChanged,
    ContentChanged,
    OpenFile { path: ProjectPath },
    Focus,
}

pub struct AgentChatView {
    content: Entity<AgentChatContent>,
    focus_handle: FocusHandle,
    workspace: WeakEntity<Workspace>,
    nav_history: Option<ItemNavHistory>,
    _subscriptions: Vec<Subscription>,
}

impl AgentChatView {
    pub fn new(
        content: Entity<AgentChatContent>,
        workspace: WeakEntity<Workspace>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();

        let subscriptions = vec![cx.subscribe(&content, Self::handle_content_event)];

        Self {
            content,
            focus_handle,
            workspace,
            nav_history: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn load(
        workspace: WeakEntity<Workspace>,
        prompt_builder: Arc<PromptBuilder>,
        cx: AsyncWindowContext,
    ) -> Task<Result<Entity<Self>>> {
        cx.spawn(async move |cx| {
            let content = AgentChatContent::load(workspace.clone(), prompt_builder, cx.clone()).await?;

            cx.update(|window, cx| {
                cx.new(|cx| Self::new(content, workspace, window, cx))
            })
        })
    }

    pub fn open(
        workspace: &mut Workspace,
        prompt_builder: Arc<PromptBuilder>,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) {
        let existing = workspace.items_of_type::<AgentChatView>(cx).next();
        if let Some(existing) = existing {
            workspace.activate_item(&existing, true, true, window, cx);
            return;
        }

        let workspace_handle = workspace.weak_handle();
        cx.spawn_in(window, async move |workspace, cx| {
            let content = AgentChatContent::load(workspace_handle.clone(), prompt_builder, cx.clone()).await?;
            workspace.update_in(cx, |workspace, window, cx| {
                let view = cx.new(|cx| AgentChatView::new(content, workspace_handle, window, cx));
                workspace.add_item_to_center(Box::new(view), window, cx);
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .detach_and_log_err(cx);
    }

    fn handle_content_event(
        &mut self,
        _content: Entity<AgentChatContent>,
        event: &AgentChatContentEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            AgentChatContentEvent::TitleChanged => {
                cx.emit(AgentChatEvent::TitleChanged);
                cx.notify();
            }
            AgentChatContentEvent::ThreadChanged => {
                cx.emit(AgentChatEvent::ContentChanged);
            }
            AgentChatContentEvent::OpenFile { path } => {
                cx.emit(AgentChatEvent::OpenFile { path: path.clone() });
            }
        }
    }

    fn title(&self, cx: &App) -> SharedString {
        self.content
            .read(cx)
            .active_thread_title(cx)
            .unwrap_or_else(|| "Agent Chat".into())
    }
}

impl EventEmitter<AgentChatEvent> for AgentChatView {}

impl Focusable for AgentChatView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for AgentChatView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .track_focus(&self.focus_handle)
            .key_context("AgentChatView")
            .on_action(cx.listener(|this, action: &crate::NewThread, window, cx| {
                this.content.update(cx, |content, cx| {
                    content.new_thread(action, window, cx);
                });
            }))
            .on_action(cx.listener(|this, _: &crate::NewTextThread, window, cx| {
                this.content.update(cx, |content, cx| {
                    content.new_text_thread(window, cx);
                });
            }))
            .on_action(cx.listener(|this, _: &crate::OpenHistory, window, cx| {
                this.content.update(cx, |content, cx| {
                    content.open_history(window, cx);
                });
            }))
            .on_action(cx.listener(|this, _: &zed_actions::agent::OpenSettings, window, cx| {
                this.content.update(cx, |content, cx| {
                    content.open_configuration(window, cx);
                });
            }))
            .on_action(cx.listener(|this, _: &workspace::GoBack, window, cx| {
                this.content.update(cx, |content, cx| {
                    content.go_back(window, cx);
                });
            }))
            .child(
                self.content.update(cx, |content, cx| {
                    content.render_toolbar(window, cx)
                })
            )
            .child(
                div()
                    .flex_1()
                    .size_full()
                    .child(
                        self.content.update(cx, |content, cx| {
                            content.render_main_content(window, cx)
                        })
                    )
            )
    }
}

impl Item for AgentChatView {
    type Event = AgentChatEvent;

    fn tab_content_text(&self, _detail: usize, cx: &App) -> SharedString {
        self.title(cx)
    }

    fn tab_content(&self, params: TabContentParams, _window: &Window, cx: &App) -> gpui::AnyElement {
        let title = self.title(cx);
        let has_unsent_message = self.content.read(cx).has_unsent_message(cx);

        h_flex()
            .gap_2()
            .child(
                Icon::new(IconName::ZedAssistant)
                    .color(if params.selected {
                        Color::Default
                    } else {
                        Color::Muted
                    })
            )
            .child(
                Label::new(title).color(if params.selected {
                    Color::Default
                } else {
                    Color::Muted
                })
            )
            .when(has_unsent_message, |this| {
                this.child(
                    div()
                        .w_2()
                        .h_2()
                        .rounded_full()
                        .bg(cx.theme().colors().icon_accent)
                )
            })
            .into_any_element()
    }

    fn tab_icon(&self, _window: &Window, _cx: &App) -> Option<Icon> {
        Some(Icon::new(IconName::ZedAssistant).color(Color::Muted))
    }

    fn tab_tooltip_text(&self, cx: &App) -> Option<SharedString> {
        Some(format!("Agent: {}", self.title(cx)).into())
    }

    fn to_item_events(event: &Self::Event, mut f: impl FnMut(ItemEvent)) {
        match event {
            AgentChatEvent::TitleChanged => f(ItemEvent::UpdateTab),
            AgentChatEvent::ContentChanged => f(ItemEvent::UpdateTab),
            _ => {}
        }
    }

    fn set_nav_history(
        &mut self,
        history: ItemNavHistory,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        self.nav_history = Some(history);
    }

    fn navigate(
        &mut self,
        _data: Box<dyn Any>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> bool {
        false
    }

    fn deactivated(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}

    fn workspace_deactivated(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}

    fn can_split(&self) -> bool {
        true
    }

    fn clone_on_split(
        &self,
        _workspace_id: Option<WorkspaceId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<Option<Entity<Self>>> {
        let content = self.content.clone();
        let workspace = self.workspace.clone();

        Task::ready(Some(cx.new(|cx| Self::new(content, workspace, window, cx))))
    }

    fn for_each_project_item(&self, _cx: &App, _f: &mut dyn FnMut(gpui::EntityId, &dyn project::ProjectItem)) {
    }

    fn buffer_kind(&self, _cx: &App) -> ItemBufferKind {
        ItemBufferKind::None
    }

    fn is_dirty(&self, cx: &App) -> bool {
        self.content.read(cx).has_unsent_message(cx)
    }

    fn has_conflict(&self, _cx: &App) -> bool {
        false
    }

    fn can_save(&self, _cx: &App) -> bool {
        false
    }

    fn as_searchable(&self, _handle: &Entity<Self>, _cx: &App) -> Option<Box<dyn SearchableItemHandle>> {
        None
    }

    fn telemetry_event_text(&self) -> Option<&'static str> {
        Some("Agent Chat Opened")
    }

    fn breadcrumbs(&self, _theme: &theme::Theme, cx: &App) -> Option<Vec<BreadcrumbText>> {
        let title = self.title(cx);
        Some(vec![
            BreadcrumbText {
                text: "Agent".to_string(),
                highlights: None,
                font: None,
            },
            BreadcrumbText {
                text: title.to_string(),
                highlights: None,
                font: None,
            },
        ])
    }
}

impl SerializableItem for AgentChatView {
    fn serialized_item_kind() -> &'static str {
        "AgentChatView"
    }

    fn cleanup(
        _workspace_id: WorkspaceId,
        _alive_items: Vec<ItemId>,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Task<Result<()>> {
        Task::ready(Ok(()))
    }

    fn deserialize(
        _project: Entity<Project>,
        _workspace: WeakEntity<Workspace>,
        _workspace_id: WorkspaceId,
        _item_id: ItemId,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Task<Result<Entity<Self>>> {
        // TODO: Implement proper deserialization
        Task::ready(Err(anyhow::anyhow!("AgentChatView deserialization not yet implemented")))
    }

    fn serialize(
        &mut self,
        _workspace: &mut Workspace,
        _item_id: ItemId,
        _closing: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Task<Result<()>>> {
        Some(Task::ready(Ok(())))
    }

    fn should_serialize(&self, _event: &Self::Event) -> bool {
        true
    }
}

pub fn register_serializable_item(cx: &mut App) {
    workspace::register_serializable_item::<AgentChatView>(cx);
}
