use std::{path::Path, sync::Arc};

use acp_thread::AgentSessionInfo;
use agent::{ContextServerRegistry, ThreadStore};
use agent_servers::AgentServer;
use anyhow::Result;
use assistant_slash_command::SlashCommandWorkingSet;
use assistant_text_thread::TextThread;
use db::kvp::KEY_VALUE_STORE;
use editor::Editor;
use fs::Fs;
use gpui::{
    AnimationExt, App, AsyncWindowContext, Entity, EventEmitter, Focusable,
    Subscription, Task, WeakEntity, Window, prelude::*,
};
use language::LanguageRegistry;
use project::Project;
use prompt_store::{PromptBuilder, PromptStore};
use serde::{Deserialize, Serialize};
use ui::{Color, ContextMenu, Label, PopoverMenuHandle, prelude::*};
use util::ResultExt as _;
use workspace::Workspace;

use crate::{
    ExternalAgent,
    acp::{AcpThreadHistory, AcpThreadView, ThreadHistoryEvent},
    agent_configuration::{AgentConfiguration, AssistantConfigurationEvent},
    text_thread_editor::{TextThreadEditor, make_lsp_adapter_delegate},
    text_thread_history::{TextThreadHistory, TextThreadHistoryEvent},
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HistoryKind {
    AgentThreads,
    TextThreads,
}

pub enum ActiveView {
    ExternalAgentThread {
        thread_view: Entity<AcpThreadView>,
    },
    TextThread {
        text_thread_editor: Entity<TextThreadEditor>,
        title_editor: Entity<editor::Editor>,
        buffer_search_bar: Entity<search::BufferSearchBar>,
        _subscriptions: Vec<Subscription>,
    },
    History {
        kind: HistoryKind,
    },
    Configuration,
}

impl ActiveView {
    pub(crate) fn claude_code(
        fs: Arc<dyn Fs>,
        thread_store: Entity<ThreadStore>,
        project: Entity<Project>,
        workspace: WeakEntity<Workspace>,
        history: Entity<AcpThreadHistory>,
        window: &mut Window,
        cx: &mut Context<AgentChatContent>,
    ) -> Self {
        let server = ExternalAgent::ClaudeCode.server(fs, thread_store.clone());
        let thread_view = cx.new(|cx| {
            AcpThreadView::new(
                server,
                None,
                None,
                workspace,
                project,
                Some(thread_store),
                None,
                history,
                false,
                window,
                cx,
            )
        });
        ActiveView::ExternalAgentThread { thread_view }
    }

    pub(crate) fn text_thread(
        text_thread_editor: Entity<TextThreadEditor>,
        _language_registry: Arc<LanguageRegistry>,
        window: &mut Window,
        cx: &mut Context<AgentChatContent>,
    ) -> Self {
        use search::BufferSearchBar;

        let buffer = text_thread_editor
            .read(cx)
            .text_thread()
            .read(cx)
            .buffer()
            .clone();
        let title_editor = cx.new(|cx| {
            Editor::for_buffer(
                buffer.clone(),
                None,
                window,
                cx,
            )
        });

        let buffer_search_bar = cx.new(|cx| BufferSearchBar::new(None, window, cx));

        Self::TextThread {
            text_thread_editor,
            title_editor,
            buffer_search_bar,
            _subscriptions: vec![],
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum AgentType {
    #[default]
    NativeAgent,
    TextThread,
    Gemini,
    ClaudeCode,
    Codex,
    Custom { name: SharedString },
}

impl AgentType {
    pub fn label(&self) -> SharedString {
        match self {
            Self::NativeAgent => "Zed Agent".into(),
            Self::TextThread => "Text Thread".into(),
            Self::Gemini => "Gemini CLI".into(),
            Self::ClaudeCode => "Claude Code".into(),
            Self::Codex => "Codex CLI".into(),
            Self::Custom { name } => name.clone(),
        }
    }

    pub fn icon(&self) -> Option<ui::IconName> {
        match self {
            Self::NativeAgent => Some(ui::IconName::ZedAgent),
            Self::TextThread => Some(ui::IconName::TextThread),
            Self::Gemini => Some(ui::IconName::AiGemini),
            Self::ClaudeCode => Some(ui::IconName::AiClaude),
            Self::Codex => Some(ui::IconName::AiOpenAi),
            Self::Custom { .. } => None,
        }
    }
}

pub enum AgentChatContentEvent {
    TitleChanged,
    ThreadChanged,
    OpenFile { path: project::ProjectPath },
}

pub struct AgentChatContent {
    pub(crate) workspace: WeakEntity<Workspace>,
    pub(crate) loading: bool,
    pub(crate) project: Entity<Project>,
    pub(crate) fs: Arc<dyn Fs>,
    pub(crate) language_registry: Arc<LanguageRegistry>,
    pub(crate) acp_history: Entity<AcpThreadHistory>,
    pub(crate) text_thread_history: Entity<TextThreadHistory>,
    pub(crate) thread_store: Entity<ThreadStore>,
    pub(crate) text_thread_store: Entity<assistant_text_thread::TextThreadStore>,
    pub(crate) prompt_store: Option<Entity<PromptStore>>,
    pub(crate) context_server_registry: Entity<ContextServerRegistry>,
    pub(crate) configuration: Option<Entity<AgentConfiguration>>,
    pub(crate) configuration_subscription: Option<Subscription>,
    pub(crate) active_view: ActiveView,
    pub(crate) previous_view: Option<ActiveView>,
    pub(crate) new_thread_menu_handle: PopoverMenuHandle<ContextMenu>,
    pub(crate) agent_panel_menu_handle: PopoverMenuHandle<ContextMenu>,
    pub(crate) agent_navigation_menu_handle: PopoverMenuHandle<ContextMenu>,
    pub(crate) agent_navigation_menu: Option<Entity<ContextMenu>>,
    pub(crate) _extension_subscription: Option<Subscription>,
    pub(crate) selected_agent: AgentType,
}

impl EventEmitter<AgentChatContentEvent> for AgentChatContent {}

impl AgentChatContent {
    pub fn load(
        workspace: WeakEntity<Workspace>,
        prompt_builder: Arc<PromptBuilder>,
        mut cx: AsyncWindowContext,
    ) -> Task<Result<Entity<Self>>> {
        let prompt_store = cx.update(|_window, cx| PromptStore::global(cx));
        cx.spawn(async move |cx| {
            let prompt_store = match prompt_store {
                Ok(prompt_store) => prompt_store.await.ok(),
                Err(_) => None,
            };

            let slash_commands = Arc::new(SlashCommandWorkingSet::default());
            let text_thread_store = workspace
                .update(cx, |workspace, cx| {
                    let project = workspace.project().clone();
                    assistant_text_thread::TextThreadStore::new(
                        project,
                        prompt_builder,
                        slash_commands,
                        cx,
                    )
                })?
                .await?;

            let content = workspace.update_in(cx, |workspace, window, cx| {
                cx.new(|cx| Self::new(workspace, text_thread_store, prompt_store, window, cx))
            })?;

            Ok(content)
        })
    }

    pub fn new(
        workspace: &Workspace,
        text_thread_store: Entity<assistant_text_thread::TextThreadStore>,
        prompt_store: Option<Entity<PromptStore>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let fs = workspace.app_state().fs.clone();
        let project = workspace.project();
        let language_registry = project.read(cx).languages().clone();
        let workspace_weak = workspace.weak_handle();

        let context_server_registry =
            cx.new(|cx| ContextServerRegistry::new(project.read(cx).context_server_store(), cx));

        let thread_store = cx.new(|cx| ThreadStore::new(cx));
        let acp_history = cx.new(|cx| AcpThreadHistory::new(None, window, cx));
        let text_thread_history =
            cx.new(|cx| TextThreadHistory::new(text_thread_store.clone(), window, cx));

        cx.subscribe_in(
            &acp_history,
            window,
            |this, _, event, window, cx| match event {
                ThreadHistoryEvent::Open(thread) => {
                    this.external_thread(
                        Some(ExternalAgent::NativeAgent),
                        Some(thread.clone()),
                        None,
                        window,
                        cx,
                    );
                }
            },
        )
        .detach();

        cx.subscribe_in(
            &text_thread_history,
            window,
            |this, _, event, window, cx| match event {
                TextThreadHistoryEvent::Open(thread) => {
                    this.open_saved_text_thread(thread.path.clone(), window, cx)
                        .detach_and_log_err(cx);
                }
            },
        )
        .detach();

        let active_view = ActiveView::claude_code(
            fs.clone(),
            thread_store.clone(),
            project.clone(),
            workspace_weak.clone(),
            acp_history.clone(),
            window,
            cx,
        );

        // Subscribe to extension events to sync agent servers when extensions change
        let extension_subscription = if let Some(extension_events) =
            extension::ExtensionEvents::try_global(cx)
        {
            Some(cx.subscribe(&extension_events, |this, _source, event, cx| match event {
                extension::Event::ExtensionInstalled(_)
                | extension::Event::ExtensionUninstalled(_)
                | extension::Event::ExtensionsInstalledChanged => {
                    this.sync_agent_servers_from_extensions(cx);
                }
                _ => {}
            }))
        } else {
            None
        };

        let mut content = Self {
            active_view,
            workspace: workspace_weak,
            project: project.clone(),
            fs: fs.clone(),
            language_registry,
            text_thread_store,
            prompt_store,
            configuration: None,
            configuration_subscription: None,
            context_server_registry,
            previous_view: None,
            new_thread_menu_handle: PopoverMenuHandle::default(),
            agent_panel_menu_handle: PopoverMenuHandle::default(),
            agent_navigation_menu_handle: PopoverMenuHandle::default(),
            agent_navigation_menu: None,
            _extension_subscription: extension_subscription,
            acp_history,
            text_thread_history,
            thread_store,
            selected_agent: AgentType::default(),
            loading: false,
        };

        content.sync_agent_servers_from_extensions(cx);
        content
    }

    pub fn active_thread_view(&self) -> Option<&Entity<AcpThreadView>> {
        match &self.active_view {
            ActiveView::ExternalAgentThread { thread_view, .. } => Some(thread_view),
            ActiveView::TextThread { .. }
            | ActiveView::History { .. }
            | ActiveView::Configuration => None,
        }
    }

    pub fn active_thread_title(&self, cx: &App) -> Option<SharedString> {
        match &self.active_view {
            ActiveView::ExternalAgentThread { thread_view } => {
                Some(thread_view.read(cx).title(cx))
            }
            ActiveView::TextThread {
                text_thread_editor, ..
            } => Some(text_thread_editor.read(cx).title(cx)),
            ActiveView::History { .. } => Some("History".into()),
            ActiveView::Configuration => Some("Configuration".into()),
        }
    }

    pub fn has_unsent_message(&self, _cx: &App) -> bool {
        // TODO: Implement actual check for unsent messages
        false
    }

    pub fn new_thread(
        &mut self,
        _action: &crate::NewThread,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.new_agent_thread(AgentType::NativeAgent, window, cx);
    }

    pub fn new_text_thread(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        telemetry::event!("Agent Thread Started", agent = "zed-text");

        let context = self
            .text_thread_store
            .update(cx, |context_store, cx| context_store.create(cx));
        let lsp_adapter_delegate = make_lsp_adapter_delegate(&self.project, cx)
            .log_err()
            .flatten();

        let text_thread_editor = cx.new(|cx| {
            let mut editor = TextThreadEditor::for_text_thread(
                context,
                self.fs.clone(),
                self.workspace.clone(),
                self.project.clone(),
                lsp_adapter_delegate,
                window,
                cx,
            );
            editor.insert_default_prompt(window, cx);
            editor
        });

        self.selected_agent = AgentType::TextThread;

        self.set_active_view(
            ActiveView::text_thread(
                text_thread_editor,
                self.language_registry.clone(),
                window,
                cx,
            ),
            true,
            window,
            cx,
        );
    }

    pub fn external_thread(
        &mut self,
        agent_choice: Option<ExternalAgent>,
        resume_thread: Option<AgentSessionInfo>,
        summarize_thread: Option<AgentSessionInfo>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let workspace = self.workspace.clone();
        let project = self.project.clone();
        let fs = self.fs.clone();
        let is_via_collab = self.project.read(cx).is_via_collab();

        const LAST_USED_EXTERNAL_AGENT_KEY: &str = "agent_panel__last_used_external_agent";

        #[derive(Serialize, Deserialize)]
        struct LastUsedExternalAgent {
            agent: ExternalAgent,
        }

        let loading = self.loading;
        let thread_store = self.thread_store.clone();

        cx.spawn_in(window, async move |this, cx| {
            let ext_agent = match agent_choice {
                Some(agent) => {
                    cx.background_spawn({
                        let agent = agent.clone();
                        async move {
                            if let Some(serialized) =
                                serde_json::to_string(&LastUsedExternalAgent { agent }).log_err()
                            {
                                KEY_VALUE_STORE
                                    .write_kvp(LAST_USED_EXTERNAL_AGENT_KEY.to_string(), serialized)
                                    .await
                                    .log_err();
                            }
                        }
                    })
                    .detach();

                    agent
                }
                None => {
                    if is_via_collab {
                        ExternalAgent::NativeAgent
                    } else {
                        cx.background_spawn(async move {
                            KEY_VALUE_STORE.read_kvp(LAST_USED_EXTERNAL_AGENT_KEY)
                        })
                        .await
                        .log_err()
                        .flatten()
                        .and_then(|value| {
                            serde_json::from_str::<LastUsedExternalAgent>(&value).log_err()
                        })
                        .map(|agent| agent.agent)
                        .unwrap_or(ExternalAgent::NativeAgent)
                    }
                }
            };

            let server = ext_agent.server(fs, thread_store);
            this.update_in(cx, |agent_content, window, cx| {
                agent_content._external_thread(
                    server,
                    resume_thread,
                    summarize_thread,
                    workspace,
                    project,
                    loading,
                    ext_agent,
                    window,
                    cx,
                );
            })?;

            anyhow::Ok(())
        })
        .detach_and_log_err(cx);
    }

    pub fn open_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(kind) = self.history_kind_for_selected_agent() else {
            return;
        };

        if let ActiveView::History { kind: active_kind } = self.active_view {
            if active_kind == kind {
                if let Some(previous_view) = self.previous_view.take() {
                    self.set_active_view(previous_view, true, window, cx);
                }
                return;
            }
        }

        self.set_active_view(ActiveView::History { kind }, true, window, cx);
        cx.notify();
    }

    pub fn open_configuration(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let ActiveView::Configuration = self.active_view {
            if let Some(previous_view) = self.previous_view.take() {
                self.set_active_view(previous_view, true, window, cx);
                return;
            }
        }

        if self.configuration.is_none() {
            let fs = self.fs.clone();
            let agent_server_store = self.project.read(cx).agent_server_store().clone();
            let context_server_store = self.project.read(cx).context_server_store();
            let context_server_registry = self.context_server_registry.clone();
            let language_registry = self.language_registry.clone();
            let workspace = self.workspace.clone();

            let configuration = cx.new(|cx| {
                AgentConfiguration::new(
                    fs,
                    agent_server_store,
                    context_server_store,
                    context_server_registry,
                    language_registry,
                    workspace,
                    window,
                    cx,
                )
            });
            self.configuration_subscription =
                Some(cx.subscribe(&configuration, Self::on_configuration_event));
            self.configuration = Some(configuration);
        }

        self.set_active_view(ActiveView::Configuration, true, window, cx);
    }

    pub fn go_back(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        match self.active_view {
            ActiveView::Configuration | ActiveView::History { .. } => {
                if let Some(previous_view) = self.previous_view.take() {
                    self.active_view = previous_view;
                }
                cx.notify();
            }
            _ => {}
        }
    }

    fn history_kind_for_selected_agent(&self) -> Option<HistoryKind> {
        match self.selected_agent {
            AgentType::NativeAgent => Some(HistoryKind::AgentThreads),
            AgentType::TextThread => Some(HistoryKind::TextThreads),
            AgentType::Gemini
            | AgentType::ClaudeCode
            | AgentType::Codex
            | AgentType::Custom { .. } => None,
        }
    }

    fn set_active_view(
        &mut self,
        new_view: ActiveView,
        save_previous: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if save_previous {
            self.previous_view = Some(std::mem::replace(&mut self.active_view, new_view));
        } else {
            self.active_view = new_view;
        }

        cx.emit(AgentChatContentEvent::ThreadChanged);
        cx.notify();
    }

    fn new_agent_thread(&mut self, agent_type: AgentType, window: &mut Window, cx: &mut Context<Self>) {
        self.selected_agent = agent_type.clone();

        match agent_type {
            AgentType::NativeAgent => {
                self.external_thread(Some(ExternalAgent::NativeAgent), None, None, window, cx)
            }
            AgentType::TextThread => self.new_text_thread(window, cx),
            AgentType::Gemini => {
                self.external_thread(Some(ExternalAgent::Gemini), None, None, window, cx)
            }
            AgentType::ClaudeCode => {
                self.external_thread(Some(ExternalAgent::ClaudeCode), None, None, window, cx)
            }
            AgentType::Codex => {
                self.external_thread(Some(ExternalAgent::Codex), None, None, window, cx)
            }
            AgentType::Custom { name } => {
                self.external_thread(
                    Some(ExternalAgent::Custom { name }),
                    None,
                    None,
                    window,
                    cx,
                )
            }
        }
    }

    fn open_saved_text_thread(
        &mut self,
        path: Arc<Path>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let text_thread_task = self
            .text_thread_store
            .update(cx, |store, cx| store.open_local(path, cx));
        cx.spawn_in(window, async move |this, cx| {
            let text_thread = text_thread_task.await?;
            this.update_in(cx, |this, window, cx| {
                this.open_text_thread(text_thread, window, cx);
            })
        })
    }

    fn open_text_thread(
        &mut self,
        text_thread: Entity<TextThread>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let lsp_adapter_delegate = make_lsp_adapter_delegate(&self.project.clone(), cx)
            .log_err()
            .flatten();
        let editor = cx.new(|cx| {
            TextThreadEditor::for_text_thread(
                text_thread,
                self.fs.clone(),
                self.workspace.clone(),
                self.project.clone(),
                lsp_adapter_delegate,
                window,
                cx,
            )
        });

        self.selected_agent = AgentType::TextThread;

        self.set_active_view(
            ActiveView::text_thread(editor, self.language_registry.clone(), window, cx),
            true,
            window,
            cx,
        );
    }

    fn on_configuration_event(
        &mut self,
        _configuration: Entity<AgentConfiguration>,
        _event: &AssistantConfigurationEvent,
        cx: &mut Context<Self>,
    ) {
        cx.notify();
    }

    fn sync_agent_servers_from_extensions(&mut self, cx: &mut Context<Self>) {
        // Sync logic would go here
        cx.notify();
    }

    pub fn render_main_content(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        match &self.active_view {
            ActiveView::ExternalAgentThread { thread_view } => {
                v_flex()
                    .size_full()
                    .child(thread_view.clone())
                    .child(self.render_drag_target(cx))
                    .into_any_element()
            }
            ActiveView::TextThread {
                text_thread_editor,
                buffer_search_bar,
                ..
            } => {
                self.render_text_thread(text_thread_editor, buffer_search_bar, _window, cx)
                    .into_any_element()
            }
            ActiveView::History { kind } => match kind {
                HistoryKind::AgentThreads => {
                    div().size_full().child(self.acp_history.clone()).into_any_element()
                }
                HistoryKind::TextThreads => {
                    div().size_full().child(self.text_thread_history.clone()).into_any_element()
                }
            }
            ActiveView::Configuration => {
                div()
                    .size_full()
                    .children(self.configuration.clone())
                    .into_any_element()
            }
        }
    }

    fn render_text_thread(
        &self,
        text_thread_editor: &Entity<TextThreadEditor>,
        buffer_search_bar: &Entity<search::BufferSearchBar>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut registrar = search::buffer_search::DivRegistrar::new(
            |this, _, _cx| match &this.active_view {
                ActiveView::TextThread {
                    buffer_search_bar, ..
                } => Some(buffer_search_bar.clone()),
                _ => None,
            },
            cx,
        );
        search::BufferSearchBar::register(&mut registrar);
        registrar
            .into_div()
            .size_full()
            .relative()
            .map(|parent| {
                buffer_search_bar.update(cx, |buffer_search_bar, cx| {
                    if buffer_search_bar.is_dismissed() {
                        return parent;
                    }
                    parent.child(
                        div()
                            .p_2()
                            .border_b_1()
                            .border_color(cx.theme().colors().border_variant)
                            .bg(cx.theme().colors().editor_background)
                            .child(buffer_search_bar.render(window, cx)),
                    )
                })
            })
            .child(text_thread_editor.clone())
            .child(self.render_drag_target(cx))
    }

    fn render_drag_target(&self, cx: &Context<Self>) -> Div {
        let is_local = self.project.read(cx).is_local();
        div()
            .invisible()
            .absolute()
            .top_0()
            .right_0()
            .bottom_0()
            .left_0()
            .bg(cx.theme().colors().drop_target_background)
            .drag_over::<workspace::DraggedTab>(|this, _, _, _| this.visible())
            .drag_over::<workspace::DraggedSelection>(|this, _, _, _| this.visible())
            .when(is_local, |this| {
                this.drag_over::<gpui::ExternalPaths>(|this, _, _, _| this.visible())
            })
            .on_drop(cx.listener(move |this, tab: &workspace::DraggedTab, window, cx| {
                let item = tab.pane.read(cx).item_for_index(tab.ix);
                let project_paths = item
                    .and_then(|item| item.project_path(cx))
                    .into_iter()
                    .collect::<Vec<_>>();
                this.handle_drop(project_paths, vec![], window, cx);
            }))
            .on_drop(
                cx.listener(move |this, selection: &workspace::DraggedSelection, window, cx| {
                    let project_paths = selection
                        .items()
                        .filter_map(|item| this.project.read(cx).path_for_entry(item.entry_id, cx))
                        .collect::<Vec<_>>();
                    this.handle_drop(project_paths, vec![], window, cx);
                }),
            )
            .on_drop(cx.listener(move |this, paths: &gpui::ExternalPaths, window, cx| {
                let tasks = paths
                    .paths()
                    .iter()
                    .map(|path| {
                        workspace::Workspace::project_path_for_path(this.project.clone(), path, false, cx)
                    })
                    .collect::<Vec<_>>();
                cx.spawn_in(window, async move |this, cx| {
                    let mut paths = vec![];
                    let mut added_worktrees = vec![];
                    let opened_paths = futures::future::join_all(tasks).await;
                    for entry in opened_paths {
                        if let Some((worktree, project_path)) = entry.log_err() {
                            added_worktrees.push(worktree);
                            paths.push(project_path);
                        }
                    }
                    this.update_in(cx, |this, window, cx| {
                        this.handle_drop(paths, added_worktrees, window, cx);
                    })
                    .ok();
                })
                .detach();
            }))
    }

    fn handle_drop(
        &mut self,
        paths: Vec<project::ProjectPath>,
        added_worktrees: Vec<Entity<project::Worktree>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match &self.active_view {
            ActiveView::ExternalAgentThread { thread_view } => {
                thread_view.update(cx, |thread_view, cx| {
                    thread_view.insert_dragged_files(paths, added_worktrees, window, cx);
                });
            }
            ActiveView::TextThread {
                text_thread_editor, ..
            } => {
                text_thread_editor.update(cx, |text_thread_editor, cx| {
                    text_thread_editor.insert_dragged_files(paths, added_worktrees, window, cx);
                });
            }
            _ => {}
        }
    }

    fn render_toolbar_back_button(&self, cx: &Context<Self>) -> impl IntoElement {
        ui::IconButton::new("go-back", ui::IconName::ArrowLeft)
            .icon_size(ui::IconSize::Small)
            .on_click(cx.listener(|this, _, window, cx| {
                this.go_back(window, cx);
            }))
            .tooltip(ui::Tooltip::text("Go Back"))
    }

    fn render_selected_agent_icon(&self, cx: &Context<Self>) -> gpui::AnyElement {
        let agent_server_store = self.project.read(cx).agent_server_store().clone();

        let (custom_icon, label) = if let AgentType::Custom { name } = &self.selected_agent {
            let store = agent_server_store.read(cx);
            let icon = store.agent_icon(&project::ExternalAgentServerName(name.clone()));
            let label = store
                .agent_display_name(&project::ExternalAgentServerName(name.clone()))
                .unwrap_or_else(|| self.selected_agent.label());
            (icon, label)
        } else {
            (None, self.selected_agent.label())
        };

        let is_loading = self
            .active_thread_view()
            .map(|tv| tv.read(cx).is_loading())
            .unwrap_or(false);

        let has_custom_icon = custom_icon.is_some();
        let selected_agent = div()
            .id("selected_agent_icon")
            .when_some(custom_icon, |this, icon_path| {
                this.px_1()
                    .child(ui::Icon::from_external_svg(icon_path).color(Color::Muted))
            })
            .when(!has_custom_icon, |this| {
                this.when_some(self.selected_agent.icon(), |this, icon| {
                    this.px_1().child(ui::Icon::new(icon).color(Color::Muted))
                })
            })
            .tooltip(move |_, cx| {
                ui::Tooltip::with_meta(label.clone(), None, "Selected Agent", cx)
            });

        if is_loading {
            selected_agent
                .with_animation(
                    "pulsating-icon",
                    gpui::Animation::new(std::time::Duration::from_secs(1))
                        .repeat()
                        .with_easing(gpui::pulsating_between(0.2, 0.6)),
                    |icon, delta| icon.opacity(delta),
                )
                .into_any_element()
        } else {
            selected_agent.into_any_element()
        }
    }

    fn render_title_view(&self, _window: &mut Window, cx: &Context<Self>) -> gpui::AnyElement {
        const LOADING_SUMMARY_PLACEHOLDER: &str = "Loading Summary…";

        let content = match &self.active_view {
            ActiveView::ExternalAgentThread { thread_view } => {
                let is_generating_title = thread_view
                    .read(cx)
                    .as_native_thread(cx)
                    .map_or(false, |t| t.read(cx).is_generating_title());

                if let Some(title_editor) = thread_view.read(cx).title_editor() {
                    let container = div()
                        .w_full()
                        .on_action({
                            let thread_view = thread_view.downgrade();
                            move |_: &menu::Confirm, window, cx| {
                                if let Some(thread_view) = thread_view.upgrade() {
                                    thread_view.focus_handle(cx).focus(window, cx);
                                }
                            }
                        })
                        .on_action({
                            let thread_view = thread_view.downgrade();
                            move |_: &editor::actions::Cancel, window, cx| {
                                if let Some(thread_view) = thread_view.upgrade() {
                                    thread_view.focus_handle(cx).focus(window, cx);
                                }
                            }
                        })
                        .child(title_editor);

                    if is_generating_title {
                        container
                            .with_animation(
                                "generating_title",
                                gpui::Animation::new(std::time::Duration::from_secs(2))
                                    .repeat()
                                    .with_easing(gpui::pulsating_between(0.4, 0.8)),
                                |div, delta| div.opacity(delta),
                            )
                            .into_any_element()
                    } else {
                        container.into_any_element()
                    }
                } else {
                    Label::new(thread_view.read(cx).title(cx))
                        .color(Color::Muted)
                        .truncate()
                        .into_any_element()
                }
            }
            ActiveView::TextThread {
                title_editor,
                text_thread_editor,
                ..
            } => {
                let summary = text_thread_editor.read(cx).text_thread().read(cx).summary();

                match summary {
                    assistant_text_thread::TextThreadSummary::Pending => Label::new(assistant_text_thread::TextThreadSummary::DEFAULT)
                        .color(Color::Muted)
                        .truncate()
                        .into_any_element(),
                    assistant_text_thread::TextThreadSummary::Content(summary) => {
                        if summary.done {
                            div()
                                .w_full()
                                .child(title_editor.clone())
                                .into_any_element()
                        } else {
                            Label::new(LOADING_SUMMARY_PLACEHOLDER)
                                .truncate()
                                .color(Color::Muted)
                                .with_animation(
                                    "generating_title",
                                    gpui::Animation::new(std::time::Duration::from_secs(2))
                                        .repeat()
                                        .with_easing(gpui::pulsating_between(0.4, 0.8)),
                                    |label, delta| label.alpha(delta),
                                )
                                .into_any_element()
                        }
                    }
                    assistant_text_thread::TextThreadSummary::Error => h_flex()
                        .w_full()
                        .child(title_editor.clone())
                        .child(
                            ui::IconButton::new("retry-summary-generation", ui::IconName::RotateCcw)
                                .icon_size(ui::IconSize::Small)
                                .on_click({
                                    let text_thread_editor = text_thread_editor.clone();
                                    move |_, _window, cx| {
                                        text_thread_editor.update(cx, |text_thread_editor, cx| {
                                            text_thread_editor.regenerate_summary(cx);
                                        });
                                    }
                                })
                                .tooltip(move |_window, cx| {
                                    cx.new(|_| {
                                        ui::Tooltip::new("Failed to generate title")
                                            .meta("Click to try again")
                                    })
                                    .into()
                                }),
                        )
                        .into_any_element(),
                }
            }
            ActiveView::History { kind } => {
                let title = match kind {
                    HistoryKind::AgentThreads => "History",
                    HistoryKind::TextThreads => "Text Threads",
                };
                Label::new(title).truncate().into_any_element()
            }
            ActiveView::Configuration => Label::new("Settings").truncate().into_any_element(),
        };

        h_flex()
            .key_context("TitleEditor")
            .id("TitleEditor")
            .flex_grow()
            .w_full()
            .max_w_full()
            .overflow_x_scroll()
            .child(content)
            .into_any()
    }

    fn render_new_thread_menu(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        use project::agent_server_store::{CLAUDE_CODE_NAME, CODEX_NAME, GEMINI_NAME};

        let agent_server_store = self.project.read(cx).agent_server_store().clone();
        let is_via_collab = self.project.read(cx).is_via_collab();
        let selected_agent = self.selected_agent.clone();
        let is_agent_selected = move |agent_type: AgentType| selected_agent == agent_type;

        let active_thread = match &self.active_view {
            ActiveView::ExternalAgentThread { thread_view } => {
                thread_view.read(cx).as_native_thread(cx)
            }
            _ => None,
        };

        ui::PopoverMenu::new("new_thread_menu")
            .trigger_with_tooltip(
                ui::IconButton::new("new_thread_menu_btn", ui::IconName::Plus)
                    .icon_size(ui::IconSize::Small),
                ui::Tooltip::text("New Thread…"),
            )
            .anchor(gpui::Corner::TopRight)
            .with_handle(self.new_thread_menu_handle.clone())
            .menu(move |window, cx| {
                telemetry::event!("New Thread Clicked");

                let active_thread = active_thread.clone();
                Some(ContextMenu::build(window, cx, |mut menu, _window, cx| {
                    menu = menu
                        .when_some(active_thread, |this, active_thread| {
                            let thread = active_thread.read(cx);

                            if !thread.is_empty() {
                                let session_id = thread.id().clone();
                                this.item(
                                    ui::ContextMenuEntry::new("New From Summary")
                                        .icon(ui::IconName::ThreadFromSummary)
                                        .icon_color(Color::Muted)
                                        .handler(move |window, cx| {
                                            window.dispatch_action(
                                                Box::new(crate::NewNativeAgentThreadFromSummary {
                                                    from_session_id: session_id.clone(),
                                                }),
                                                cx,
                                            );
                                        }),
                                )
                            } else {
                                this
                            }
                        })
                        .item(
                            ui::ContextMenuEntry::new("Zed Agent")
                                .when(is_agent_selected(AgentType::NativeAgent) | is_agent_selected(AgentType::TextThread), |this| {
                                    this.action(Box::new(crate::NewExternalAgentThread { agent: None }))
                                })
                                .icon(ui::IconName::ZedAgent)
                                .icon_color(Color::Muted)
                                .handler(move |window, cx| {
                                    window.dispatch_action(
                                        Box::new(crate::NewExternalAgentThread {
                                            agent: Some(crate::ExternalAgent::NativeAgent),
                                        }),
                                        cx,
                                    );
                                }),
                        )
                        .item(
                            ui::ContextMenuEntry::new("Text Thread")
                                .action(Box::new(crate::NewTextThread))
                                .icon(ui::IconName::TextThread)
                                .icon_color(Color::Muted)
                                .handler(move |window, cx| {
                                    window.dispatch_action(
                                        Box::new(crate::NewTextThread),
                                        cx,
                                    );
                                }),
                        )
                        .separator()
                        .header("External Agents")
                        .item(
                            ui::ContextMenuEntry::new("Claude Code")
                                .when(is_agent_selected(AgentType::ClaudeCode), |this| {
                                    this.action(Box::new(crate::NewExternalAgentThread { agent: None }))
                                })
                                .icon(ui::IconName::AiClaude)
                                .disabled(is_via_collab)
                                .icon_color(Color::Muted)
                                .handler(move |window, cx| {
                                    window.dispatch_action(
                                        Box::new(crate::NewExternalAgentThread {
                                            agent: Some(crate::ExternalAgent::ClaudeCode),
                                        }),
                                        cx,
                                    );
                                }),
                        )
                        .item(
                            ui::ContextMenuEntry::new("Codex CLI")
                                .when(is_agent_selected(AgentType::Codex), |this| {
                                    this.action(Box::new(crate::NewExternalAgentThread { agent: None }))
                                })
                                .icon(ui::IconName::AiOpenAi)
                                .disabled(is_via_collab)
                                .icon_color(Color::Muted)
                                .handler(move |window, cx| {
                                    window.dispatch_action(
                                        Box::new(crate::NewExternalAgentThread {
                                            agent: Some(crate::ExternalAgent::Codex),
                                        }),
                                        cx,
                                    );
                                }),
                        )
                        .item(
                            ui::ContextMenuEntry::new("Gemini CLI")
                                .when(is_agent_selected(AgentType::Gemini), |this| {
                                    this.action(Box::new(crate::NewExternalAgentThread { agent: None }))
                                })
                                .icon(ui::IconName::AiGemini)
                                .icon_color(Color::Muted)
                                .disabled(is_via_collab)
                                .handler(move |window, cx| {
                                    window.dispatch_action(
                                        Box::new(crate::NewExternalAgentThread {
                                            agent: Some(crate::ExternalAgent::Gemini),
                                        }),
                                        cx,
                                    );
                                }),
                        );

                    // Add custom agent servers
                    let agent_server_store = agent_server_store.read(cx);
                    let agent_names = agent_server_store
                        .external_agents()
                        .filter(|name| {
                            name.0 != GEMINI_NAME
                                && name.0 != CLAUDE_CODE_NAME
                                && name.0 != CODEX_NAME
                        })
                        .cloned()
                        .collect::<Vec<_>>();

                    for agent_name in agent_names {
                        let icon_path = agent_server_store.agent_icon(&agent_name);
                        let display_name = agent_server_store
                            .agent_display_name(&agent_name)
                            .unwrap_or_else(|| agent_name.0.clone());

                        let mut entry = ui::ContextMenuEntry::new(display_name);

                        if let Some(icon_path) = icon_path {
                            entry = entry.custom_icon_svg(icon_path);
                        } else {
                            entry = entry.icon(ui::IconName::Sparkle);
                        }

                        entry = entry.icon_color(Color::Muted);

                        let agent_name_for_handler = agent_name.clone();
                        entry = entry.handler(move |window, cx| {
                            window.dispatch_action(
                                Box::new(crate::NewExternalAgentThread {
                                    agent: Some(crate::ExternalAgent::Custom {
                                        name: agent_name_for_handler.0.clone(),
                                    }),
                                }),
                                cx,
                            );
                        });

                        menu = menu.item(entry);
                    }

                    menu = menu
                        .separator()
                        .item(
                            ui::ContextMenuEntry::new("Add More Agents")
                                .icon(ui::IconName::Plus)
                                .icon_color(Color::Muted)
                                .handler(move |window, cx| {
                                    window.dispatch_action(
                                        Box::new(zed_actions::Extensions {
                                            category_filter: Some(
                                                zed_actions::ExtensionCategoryFilter::AgentServers,
                                            ),
                                            id: None,
                                        }),
                                        cx,
                                    );
                                }),
                        );

                    menu
                }))
            })
    }

    fn render_panel_options_menu(
        &self,
        _window: &mut Window,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let usage: Option<client::RequestUsage> = None;
        let account_url = client::zed_urls::account_url(cx);

        let selected_agent = self.selected_agent.clone();

        let text_thread_view = match &self.active_view {
            ActiveView::TextThread {
                text_thread_editor, ..
            } => Some(text_thread_editor.clone()),
            _ => None,
        };
        let text_thread_with_messages = match &self.active_view {
            ActiveView::TextThread {
                text_thread_editor, ..
            } => text_thread_editor
                .read(cx)
                .text_thread()
                .read(cx)
                .messages(cx)
                .any(|message| message.role == language_model::Role::Assistant),
            _ => false,
        };

        let thread_view = match &self.active_view {
            ActiveView::ExternalAgentThread { thread_view } => Some(thread_view.clone()),
            _ => None,
        };
        let thread_with_messages = match &self.active_view {
            ActiveView::ExternalAgentThread { thread_view } => {
                thread_view.read(cx).has_user_submitted_prompt(cx)
            }
            _ => false,
        };

        ui::PopoverMenu::new("agent-options-menu")
            .trigger_with_tooltip(
                ui::IconButton::new("agent-options-menu", ui::IconName::Ellipsis)
                    .icon_size(ui::IconSize::Small),
                ui::Tooltip::text("Toggle Agent Menu"),
            )
            .anchor(gpui::Corner::TopRight)
            .with_handle(self.agent_panel_menu_handle.clone())
            .menu(move |_window, cx| {
                Some(ContextMenu::build(_window, cx, |mut menu, _window, _| {
                    if let Some(usage) = usage {
                        menu = menu
                            .header_with_link("Prompt Usage", "Manage", account_url.clone())
                            .custom_entry(
                                move |_window, cx| {
                                    use cloud_llm_client::UsageLimit;

                                    let used_percentage = match usage.limit {
                                        UsageLimit::Limited(limit) => {
                                            Some((usage.amount as f32 / limit as f32) * 100.)
                                        }
                                        UsageLimit::Unlimited => None,
                                    };

                                    h_flex()
                                        .flex_1()
                                        .gap_1p5()
                                        .children(used_percentage.map(|percent| {
                                            ui::ProgressBar::new("usage", percent, 100., cx)
                                        }))
                                        .child(
                                            Label::new(match usage.limit {
                                                UsageLimit::Limited(limit) => {
                                                    format!("{} / {limit}", usage.amount)
                                                }
                                                UsageLimit::Unlimited => {
                                                    format!("{} / ∞", usage.amount)
                                                }
                                            })
                                            .size(ui::LabelSize::Small)
                                            .color(Color::Muted),
                                        )
                                        .into_any_element()
                                },
                                move |_, cx| cx.open_url(&client::zed_urls::account_url(cx)),
                            )
                            .separator()
                    }

                    if thread_with_messages | text_thread_with_messages {
                        menu = menu.header("Current Thread");

                        if let Some(text_thread_view) = text_thread_view.as_ref() {
                            menu = menu
                                .entry("Regenerate Thread Title", None, {
                                    let text_thread_view = text_thread_view.clone();
                                    move |_, cx| {
                                        text_thread_view.update(cx, |editor, cx| {
                                            editor.regenerate_summary(cx);
                                        });
                                    }
                                })
                                .separator();
                        }

                        if let Some(thread_view) = thread_view.as_ref() {
                            menu = menu
                                .entry("Regenerate Thread Title", None, {
                                    let thread_view = thread_view.clone();
                                    move |_, cx| {
                                        if let Some(native_thread) =
                                            thread_view.read(cx).as_native_thread(cx)
                                        {
                                            native_thread.update(cx, |thread, cx| {
                                                thread.generate_title(cx);
                                            });
                                        }
                                    }
                                })
                                .separator();
                        }
                    }

                    menu = menu
                        .header("MCP Servers")
                        .action(
                            "View Server Extensions",
                            Box::new(zed_actions::Extensions {
                                category_filter: Some(
                                    zed_actions::ExtensionCategoryFilter::ContextServers,
                                ),
                                id: None,
                            }),
                        )
                        .action("Add Custom Server…", Box::new(crate::AddContextServer))
                        .separator()
                        .action("Rules", Box::new(zed_actions::assistant::OpenRulesLibrary::default()))
                        .action("Profiles", Box::new(crate::ManageProfiles::default()))
                        .action("Settings", Box::new(zed_actions::agent::OpenSettings))
                        .separator();

                    if selected_agent == AgentType::Gemini {
                        menu = menu.action("Reauthenticate", Box::new(zed_actions::agent::ReauthenticateAgent))
                    }

                    menu
                }))
            })
    }

    fn render_recent_entries_menu(
        &self,
        icon: ui::IconName,
        corner: gpui::Corner,
        _cx: &Context<Self>,
    ) -> impl IntoElement {
        ui::PopoverMenu::new("agent-nav-menu")
            .trigger_with_tooltip(
                ui::IconButton::new("agent-nav-menu", icon).icon_size(ui::IconSize::Small),
                ui::Tooltip::text("Toggle Recently Updated Threads"),
            )
            .anchor(corner)
            .with_handle(self.agent_navigation_menu_handle.clone())
            .menu({
                let menu = self.agent_navigation_menu.clone();
                move |_window, cx| {
                    telemetry::event!("View Thread History Clicked");

                    if let Some(menu) = menu.as_ref() {
                        menu.update(cx, |_, cx| {
                            cx.defer_in(_window, |menu, window, cx| {
                                menu.rebuild(window, cx);
                            });
                        })
                    }
                    menu.clone()
                }
            })
    }

    pub fn render_toolbar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let show_history_menu = self.history_kind_for_selected_agent().is_some();

        let left_section = h_flex()
            .size_full()
            .gap_2()
            .child(match &self.active_view {
                ActiveView::History { .. } | ActiveView::Configuration => {
                    self.render_toolbar_back_button(cx).into_any_element()
                }
                _ => self.render_selected_agent_icon(cx),
            })
            .child(self.render_title_view(window, cx));

        let right_section = h_flex()
            .flex_none()
            .gap_2()
            .px_2()
            .child(self.render_new_thread_menu(window, cx))
            .when(show_history_menu, |this| {
                this.child(self.render_recent_entries_menu(
                    ui::IconName::MenuAltTemp,
                    gpui::Corner::TopRight,
                    cx,
                ))
            })
            .child(self.render_panel_options_menu(window, cx));

        h_flex()
            .h_10()
            .w_full()
            .flex_none()
            .bg(cx.theme().colors().tab_bar_background)
            .border_b_1()
            .border_color(cx.theme().colors().border)
            .child(left_section)
            .child(right_section)
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn _external_thread(
        &mut self,
        server: std::rc::Rc<dyn AgentServer>,
        resume_thread: Option<AgentSessionInfo>,
        summarize_thread: Option<AgentSessionInfo>,
        workspace: WeakEntity<Workspace>,
        project: Entity<Project>,
        loading: bool,
        _ext_agent: ExternalAgent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let history = self.acp_history.clone();
        let thread_view = cx.new(|cx| {
            AcpThreadView::new(
                server,
                resume_thread,
                summarize_thread,
                workspace.clone(),
                project.clone(),
                Some(self.thread_store.clone()),
                self.prompt_store.clone(),
                history,
                !loading,
                window,
                cx,
            )
        });

        let view = ActiveView::ExternalAgentThread { thread_view };

        self.set_active_view(view, !loading, window, cx);
    }
}
