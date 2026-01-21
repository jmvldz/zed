use std::any::Any;
use std::sync::Arc;

use anyhow::{Context as AnyhowContext, Result};
use gpui::{
    App, AsyncWindowContext, Context, Entity, EventEmitter, FocusHandle, Focusable,
    SharedString, Subscription, Task, WeakEntity, Window, prelude::*,
};
use project::{Project, ProjectPath};
use prompt_store::PromptBuilder;
use serde::{Deserialize, Serialize};
use ui::{prelude::*, Color, Icon, IconName, Label};
use workspace::{
    AppState, Item, ItemId, ItemNavHistory, SerializableItem, Workspace, WorkspaceId,
    delete_unloaded_items,
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
            .on_action(cx.listener(|this, action: &crate::NewExternalAgentThread, window, cx| {
                this.content.update(cx, |content, cx| {
                    content.external_thread(
                        action.agent.clone(),
                        None,
                        None,
                        window,
                        cx,
                    );
                });
            }))
            .on_action(cx.listener(|this, action: &crate::NewNativeAgentThreadFromSummary, window, cx| {
                let from_session_id = action.from_session_id.clone();
                this.content.update(cx, |content, cx| {
                    let thread = content
                        .thread_store
                        .read(cx)
                        .thread_from_session_id(&from_session_id);

                    if let Some(thread) = thread {
                        let session_info = acp_thread::AgentSessionInfo {
                            session_id: thread.id.clone(),
                            cwd: None,
                            title: Some(thread.title.clone()),
                            updated_at: Some(thread.updated_at),
                            meta: None,
                        };
                        content.external_thread(
                            Some(crate::ExternalAgent::NativeAgent),
                            None,
                            Some(session_info),
                            window,
                            cx,
                        );
                    }
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
        _data: Arc<dyn Any + Send>,
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
        workspace_id: WorkspaceId,
        alive_items: Vec<ItemId>,
        _window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<()>> {
        delete_unloaded_items(
            alive_items,
            workspace_id,
            "agent_chat_views",
            &persistence::AGENT_CHAT_VIEW_DB,
            cx,
        )
    }

    fn deserialize(
        _project: Entity<Project>,
        _workspace: WeakEntity<Workspace>,
        _workspace_id: WorkspaceId,
        _item_id: ItemId,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Task<Result<Entity<Self>>> {
        let workspace = _workspace.clone();
        let workspace_id = _workspace_id;
        let item_id = _item_id;
        _window.spawn(_cx, async move |cx| {
            let serialized = persistence::AGENT_CHAT_VIEW_DB
                .get_state(item_id, workspace_id)
                .context("Failed to load agent chat view state")?;

            let prompt_builder = cx.update(|_window, cx| {
                let app_state = AppState::global(cx)
                    .upgrade()
                    .context("app state gone")?;
                anyhow::Ok(PromptBuilder::load(app_state.fs.clone(), false, cx))
            })??;

            let content = AgentChatContent::load(workspace.clone(), prompt_builder, cx.clone())
                .await?;

            let view = cx.update(|window, cx| {
                let view = cx.new(|cx| Self::new(content, workspace, window, cx));
                if let Some(selected_agent) = serialized.and_then(|state| state.selected_agent) {
                    view.update(cx, |view, cx| {
                        view.content.update(cx, |content, cx| {
                            content.restore_agent(selected_agent, window, cx);
                        });
                    });
                }
                view
            })?;

            Ok(view)
        })
    }

    fn serialize(
        &mut self,
        _workspace: &mut Workspace,
        _item_id: ItemId,
        _closing: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Task<Result<()>>> {
        let workspace_id = _workspace.database_id()?;
        let selected_agent = self.content.read(_cx).selected_agent.clone();
        let state = SerializedAgentChatView {
            selected_agent: Some(selected_agent),
        };

        Some(_cx.background_spawn(async move {
            persistence::AGENT_CHAT_VIEW_DB
                .save_state(_item_id, workspace_id, state)
                .await
        }))
    }

    fn should_serialize(&self, _event: &Self::Event) -> bool {
        true
    }
}

pub fn register_serializable_item(cx: &mut App) {
    workspace::register_serializable_item::<AgentChatView>(cx);
}

#[derive(Debug, Serialize, Deserialize)]
struct SerializedAgentChatView {
    selected_agent: Option<crate::agent_chat_content::AgentType>,
}

mod persistence {
    use super::SerializedAgentChatView;
    use anyhow::Context as _;
    use db::{
        sqlez::{domain::Domain, thread_safe_connection::ThreadSafeConnection},
        sqlez_macros::sql,
    };
    use workspace::{ItemId, WorkspaceDb, WorkspaceId};

    pub struct AgentChatViewDb(ThreadSafeConnection);

    impl Domain for AgentChatViewDb {
        const NAME: &str = stringify!(AgentChatViewDb);

        const MIGRATIONS: &[&str] = &[sql!(
            CREATE TABLE agent_chat_views(
                workspace_id INTEGER,
                item_id INTEGER UNIQUE,
                selected_agent TEXT,
                PRIMARY KEY(workspace_id, item_id),
                FOREIGN KEY(workspace_id) REFERENCES workspaces(workspace_id)
                ON DELETE CASCADE
            ) STRICT;
        )];
    }

    db::static_connection!(AGENT_CHAT_VIEW_DB, AgentChatViewDb, [WorkspaceDb]);

    impl AgentChatViewDb {
        pub async fn save_state(
            &self,
            item_id: ItemId,
            workspace_id: WorkspaceId,
            state: SerializedAgentChatView,
        ) -> anyhow::Result<()> {
            self.write(move |connection| {
                let sql_stmt = sql!(
                    INSERT OR REPLACE INTO agent_chat_views(item_id, workspace_id, selected_agent)
                    VALUES (?, ?, ?)
                );
                let selected_agent = serde_json::to_string(&state.selected_agent)?;
                let mut query =
                    connection.exec_bound::<(ItemId, WorkspaceId, String)>(sql_stmt)?;
                query((item_id, workspace_id, selected_agent)).context(format!(
                    "exec_bound failed to execute or parse for: {}",
                    sql_stmt
                ))
            })
            .await
        }

        pub fn get_state(
            &self,
            item_id: ItemId,
            workspace_id: WorkspaceId,
        ) -> anyhow::Result<Option<SerializedAgentChatView>> {
            let sql_stmt = sql!(
                SELECT selected_agent FROM agent_chat_views WHERE item_id = ? AND workspace_id = ?
            );
            let selected_agent_str =
                self.select_row_bound::<(ItemId, WorkspaceId), String>(sql_stmt)?(
                    (item_id, workspace_id),
                )
                .context(format!(
                    "Error in get_state, select_row_bound failed to execute or parse for: {}",
                    sql_stmt
                ))?;
            let Some(selected_agent_str) = selected_agent_str else {
                return Ok(None);
            };
            let selected_agent =
                serde_json::from_str::<Option<crate::agent_chat_content::AgentType>>(
                    &selected_agent_str,
                )?;
            Ok(Some(SerializedAgentChatView { selected_agent }))
        }
    }
}
