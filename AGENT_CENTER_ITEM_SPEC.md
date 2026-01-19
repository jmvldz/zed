# Implementation Spec: Agent Chat as Center Item

## Executive Summary

This spec details the implementation of an **AgentChatView** that opens as a center pane item (like a file), making the agent chat the primary workspace experience. Code files open as splits beside it.

**Target UI:**
```
┌─────────────────────────────────────────────────────────────────────┐
│ Tabs: [● Agent Chat ▼] [main.rs] [lib.rs]                           │
├──────────────────────────────────┬──────────────────────────────────┤
│         AGENT CHAT VIEW          │        EDITOR (Split)            │
│  ┌────────────┬─────────────┐    │   ┌──────────────────────────┐   │
│  │  Session   │   Thread    │    │   │  main.rs                 │   │
│  │  History   │   Content   │    │   │                          │   │
│  │            │             │    │   │  fn main() {             │   │
│  │  Today     │  [Messages] │    │   │      println!("Hi");     │   │
│  │  • Thread1 │             │    │   │  }                       │   │
│  │  • Thread2 │  [Input]    │    │   │                          │   │
│  │            │             │    │   └──────────────────────────┘   │
│  └────────────┴─────────────┘    │                                  │
└──────────────────────────────────┴──────────────────────────────────┘
           ~60% width                        ~40% width
```

---

## Table of Contents

1. [Architecture Overview](#1-architecture-overview)
2. [New Types and Structures](#2-new-types-and-structures)
3. [Extracting Shared Components](#3-extracting-shared-components)
4. [AgentChatView Implementation](#4-agentchatview-implementation)
5. [Item Trait Implementation](#5-item-trait-implementation)
6. [Actions and Keybindings](#6-actions-and-keybindings)
7. [Startup Behavior](#7-startup-behavior)
8. [File Opening Behavior](#8-file-opening-behavior)
9. [Settings](#9-settings)
10. [Serialization and Persistence](#10-serialization-and-persistence)
11. [Navigation and Focus](#11-navigation-and-focus)
12. [Relationship with AgentPanel](#12-relationship-with-agentpanel)
13. [Implementation Phases](#13-implementation-phases)
14. [Testing Strategy](#14-testing-strategy)
15. [File Changes Summary](#15-file-changes-summary)

---

## 1. Architecture Overview

### Current Architecture

```
AgentPanel (dock panel)
    ├── ActiveView (enum)
    │   ├── ExternalAgentThread { thread_view: AcpThreadView }
    │   ├── TextThread { ... }
    │   ├── History { kind: HistoryKind }
    │   └── Configuration
    ├── AcpThreadHistory (session list)
    ├── TextThreadHistory
    └── AgentConfiguration
```

### New Architecture

```
AgentChatView (center item) ◄── NEW
    ├── AgentChatContent (shared) ◄── EXTRACTED
    │   ├── ActiveView
    │   ├── AcpThreadHistory
    │   ├── TextThreadHistory
    │   └── AgentConfiguration
    └── Implements workspace::Item

AgentPanel (dock panel, optional for backwards compat)
    └── Uses AgentChatContent (shared)
```

### Key Insight

The `AgentPanel` currently contains ALL the logic for:
- Thread management
- History display
- Configuration
- Toolbar rendering
- Drag/drop
- Zoom
- etc.

We will **extract the core content** into `AgentChatContent`, then both `AgentChatView` (center) and `AgentPanel` (dock) can use it.

---

## 2. New Types and Structures

### 2.1 AgentChatView

**File**: `crates/agent_ui/src/agent_chat_view.rs` (NEW)

```rust
/// A center pane view for agent chat.
/// Opens as a tab in the workspace, making agent chat the primary experience.
pub struct AgentChatView {
    /// The shared content (thread view, history, etc.)
    content: Entity<AgentChatContent>,

    /// Focus handle for this item
    focus_handle: FocusHandle,

    /// Weak reference to workspace
    workspace: WeakEntity<Workspace>,

    /// Navigation history for back/forward
    nav_history: Option<ItemNavHistory>,

    /// Subscriptions to content events
    _subscriptions: Vec<Subscription>,
}
```

### 2.2 AgentChatContent

**File**: `crates/agent_ui/src/agent_chat_content.rs` (NEW)

```rust
/// The core agent chat content, shared between AgentChatView and AgentPanel.
/// Contains all the logic for managing threads, history, and configuration.
pub struct AgentChatContent {
    // Carried over from AgentPanel
    pub(crate) workspace: WeakEntity<Workspace>,
    pub(crate) loading: bool,
    pub(crate) user_store: Entity<UserStore>,
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
    pub(crate) onboarding: Entity<AgentPanelOnboarding>,
    pub(crate) selected_agent: AgentType,
    pub(crate) show_trust_workspace_message: bool,

    // NEW: Whether to show session history sidebar
    pub(crate) show_history_sidebar: bool,
}
```

### 2.3 AgentChatEvent

**File**: `crates/agent_ui/src/agent_chat_view.rs`

```rust
/// Events emitted by AgentChatView for the workspace to handle
pub enum AgentChatEvent {
    /// Thread title changed (update tab)
    TitleChanged,
    /// Thread content changed
    ContentChanged,
    /// User wants to open a file (for split behavior)
    OpenFile { path: ProjectPath },
    /// Focus requested
    Focus,
}
```

---

## 3. Extracting Shared Components

### 3.1 Move Core Logic to AgentChatContent

**Current location**: `crates/agent_ui/src/agent_panel.rs`

**Methods to move to AgentChatContent**:

```rust
impl AgentChatContent {
    // === Initialization ===
    pub fn new(...) -> Self { ... }
    pub fn load(...) -> Task<Result<Entity<Self>>> { ... }

    // === Thread Management ===
    pub fn new_thread(&mut self, action: &NewThread, window, cx) { ... }
    pub fn new_text_thread(&mut self, window, cx) { ... }
    pub fn external_thread(&mut self, agent, session_id, cwd, window, cx) { ... }
    pub fn new_native_agent_thread_from_summary(&mut self, ...) { ... }

    // === View Navigation ===
    pub fn open_history(&mut self, window, cx) { ... }
    pub fn open_configuration(&mut self, window, cx) { ... }
    pub fn go_back(&mut self, window, cx) { ... }

    // === Active Thread Access ===
    pub fn active_thread(&self) -> Option<&Entity<AcpThread>> { ... }
    pub fn active_thread_view(&self) -> Option<&Entity<AcpThreadView>> { ... }

    // === Rendering (return impl IntoElement, not full panel) ===
    pub fn render_content(&mut self, window, cx) -> impl IntoElement { ... }
    pub fn render_toolbar(&self, window, cx) -> impl IntoElement { ... }
    pub fn render_history_sidebar(&self, window, cx) -> impl IntoElement { ... }

    // === Settings/Font ===
    pub fn increase_font_size(&mut self, window, cx) { ... }
    pub fn decrease_font_size(&mut self, window, cx) { ... }
    pub fn reset_font_size(&mut self, window, cx) { ... }

    // === Menus ===
    pub fn toggle_navigation_menu(&mut self, window, cx) { ... }
    pub fn toggle_options_menu(&mut self, window, cx) { ... }
    pub fn toggle_new_thread_menu(&mut self, window, cx) { ... }

    // === Serialization ===
    fn serialize(&mut self, cx) { ... }
}
```

### 3.2 Refactor AgentPanel to Use AgentChatContent

**File**: `crates/agent_ui/src/agent_panel.rs`

```rust
pub struct AgentPanel {
    /// Shared content with AgentChatView
    content: Entity<AgentChatContent>,

    /// Panel-specific state
    width: Option<Pixels>,
    height: Option<Pixels>,
    zoomed: bool,
    pending_serialization: Option<Task<Result<()>>>,
}

impl AgentPanel {
    pub fn load(...) -> Task<Result<Entity<Self>>> {
        // Load AgentChatContent, then wrap in AgentPanel
        let content = AgentChatContent::load(...).await?;
        cx.new(|cx| AgentPanel {
            content,
            width: serialized.width,
            height: None,
            zoomed: false,
            pending_serialization: None,
        })
    }
}

impl Render for AgentPanel {
    fn render(&mut self, window, cx) -> impl IntoElement {
        // Delegate to content with panel-specific wrapping
        self.content.update(cx, |content, cx| {
            content.render_content(window, cx)
        })
    }
}
```

---

## 4. AgentChatView Implementation

### 4.1 Full Implementation

**File**: `crates/agent_ui/src/agent_chat_view.rs`

```rust
use gpui::*;
use workspace::{Item, ItemHandle, ItemNavHistory, Workspace};
use crate::agent_chat_content::{AgentChatContent, AgentChatContentEvent};

pub struct AgentChatView {
    content: Entity<AgentChatContent>,
    focus_handle: FocusHandle,
    workspace: WeakEntity<Workspace>,
    nav_history: Option<ItemNavHistory>,
    _subscriptions: Vec<Subscription>,
}

impl AgentChatView {
    /// Create a new agent chat view
    pub fn new(
        content: Entity<AgentChatContent>,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();

        // Subscribe to content events to update tab title, etc.
        let subscriptions = vec![
            cx.subscribe(&content, Self::handle_content_event),
        ];

        Self {
            content,
            focus_handle,
            workspace,
            nav_history: None,
            _subscriptions: subscriptions,
        }
    }

    /// Load and create a new agent chat view
    pub fn load(
        workspace: WeakEntity<Workspace>,
        prompt_builder: Arc<PromptBuilder>,
        cx: AsyncWindowContext,
    ) -> Task<Result<Entity<Self>>> {
        cx.spawn(async move |mut cx| {
            let content = AgentChatContent::load(
                workspace.clone(),
                prompt_builder,
                cx.clone(),
            ).await?;

            cx.new(|cx| Self::new(content, workspace, window, cx))
        })
    }

    /// Open an agent chat in the workspace
    pub fn open(
        workspace: &mut Workspace,
        prompt_builder: Arc<PromptBuilder>,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) {
        // Check if there's already an agent chat open
        if let Some(existing) = workspace.items_of_type::<AgentChatView>(cx).next() {
            workspace.activate_item(&existing, true, true, window, cx);
            return;
        }

        // Load and add new agent chat
        let task = Self::load(workspace.weak_handle(), prompt_builder, cx.to_async());
        cx.spawn_in(window, async move |workspace, cx| {
            let view = task.await?;
            workspace.update_in(cx, |workspace, window, cx| {
                workspace.add_item_to_center(Box::new(view), window, cx);
            })?;
            Ok(())
        }).detach_and_log_err(cx);
    }

    /// Open an agent chat, creating a split with the current item
    pub fn open_as_split(
        workspace: &mut Workspace,
        prompt_builder: Arc<PromptBuilder>,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) {
        let task = Self::load(workspace.weak_handle(), prompt_builder, cx.to_async());
        cx.spawn_in(window, async move |workspace, cx| {
            let view = task.await?;
            workspace.update_in(cx, |workspace, window, cx| {
                // Split left so agent is on left, current item on right
                workspace.split_item(SplitDirection::Left, Box::new(view), window, cx);
            })?;
            Ok(())
        }).detach_and_log_err(cx);
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
                // Emit event for workspace to handle
                cx.emit(AgentChatEvent::OpenFile { path: path.clone() });
            }
        }
    }

    /// Get current thread title for tab
    fn title(&self, cx: &App) -> SharedString {
        self.content.read(cx).active_thread_title()
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
        let content = self.content.clone();

        // Full-width layout with optional history sidebar
        h_flex()
            .size_full()
            .track_focus(&self.focus_handle)
            .key_context("AgentChatView")
            // Actions
            .on_action(cx.listener(|this, action: &NewThread, window, cx| {
                this.content.update(cx, |content, cx| {
                    content.new_thread(action, window, cx);
                });
            }))
            .on_action(cx.listener(|this, _: &OpenHistory, window, cx| {
                this.content.update(cx, |content, cx| {
                    content.open_history(window, cx);
                });
            }))
            // ... other actions ...
            .child(
                // History sidebar (collapsible)
                content.update(cx, |content, cx| {
                    content.render_history_sidebar(window, cx)
                })
            )
            .child(
                // Main content area
                content.update(cx, |content, cx| {
                    content.render_main_content(window, cx)
                })
            )
    }
}
```

---

## 5. Item Trait Implementation

### 5.1 Full Item Implementation

**File**: `crates/agent_ui/src/agent_chat_view.rs`

```rust
impl Item for AgentChatView {
    type Event = AgentChatEvent;

    // === Tab Content ===

    fn tab_content_text(&self, _detail: usize, cx: &App) -> SharedString {
        self.title(cx)
    }

    fn tab_content(&self, params: TabContentParams, _window: &Window, cx: &App) -> AnyElement {
        let title = self.title(cx);
        let has_unsent_message = self.content.read(cx).has_unsent_message();

        h_flex()
            .gap_2()
            .child(
                Icon::new(IconName::ZedAssistant)
                    .color(if params.selected { Color::Default } else { Color::Muted })
            )
            .child(
                Label::new(title)
                    .color(if params.selected { Color::Default } else { Color::Muted })
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

    // === Events ===

    fn to_item_events(event: &Self::Event, mut f: impl FnMut(ItemEvent)) {
        match event {
            AgentChatEvent::TitleChanged => f(ItemEvent::UpdateTab),
            AgentChatEvent::ContentChanged => f(ItemEvent::UpdateTab),
            AgentChatEvent::Focus => f(ItemEvent::Focus),
            _ => {}
        }
    }

    // === Navigation ===

    fn set_nav_history(&mut self, history: ItemNavHistory, _window: &mut Window, _cx: &mut Context<Self>) {
        self.nav_history = Some(history);
    }

    fn navigate(&mut self, data: Arc<dyn Any + Send>, window: &mut Window, cx: &mut Context<Self>) -> bool {
        // Navigate to a specific thread or view
        if let Some(nav_data) = data.downcast_ref::<AgentChatNavData>() {
            self.content.update(cx, |content, cx| {
                content.navigate_to(nav_data, window, cx)
            });
            true
        } else {
            false
        }
    }

    // === Lifecycle ===

    fn deactivated(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        // Called when tab loses focus
    }

    fn workspace_deactivated(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        // Called when workspace window loses focus
    }

    // === Splitting ===

    fn can_split(&self, _cx: &App) -> bool {
        true  // Allow splitting with editors
    }

    fn clone_on_split(
        &self,
        _workspace_id: Option<WorkspaceId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<Option<Entity<Self>>> {
        // Create a new agent chat pointing to same thread
        let content = self.content.clone();
        let workspace = self.workspace.clone();

        Task::ready(Some(cx.new(|cx| {
            Self::new(content, workspace, window, cx)
        })))
    }

    // === Project Integration ===

    fn for_each_project_item(&self, _cx: &App, _f: &mut dyn FnMut(EntityId, &dyn ProjectItem)) {
        // Agent chat doesn't have project items in the traditional sense
    }

    fn buffer_kind(&self, _cx: &App) -> ItemBufferKind {
        ItemBufferKind::Unspecified
    }

    // === Save (for drafts) ===

    fn is_dirty(&self, cx: &App) -> bool {
        self.content.read(cx).has_unsent_message()
    }

    fn has_conflict(&self, _cx: &App) -> bool {
        false
    }

    fn can_save(&self, _cx: &App) -> bool {
        false  // Threads auto-save
    }

    // === Search ===

    fn as_searchable(&self, _handle: &Entity<Self>, _cx: &App) -> Option<Box<dyn SearchableItemHandle>> {
        // Could implement search within chat history
        None
    }

    // === Telemetry ===

    fn telemetry_event_text(&self) -> Option<&'static str> {
        Some("Agent Chat Opened")
    }

    // === Breadcrumbs ===

    fn breadcrumbs(&self, _theme: &theme::Theme, cx: &App) -> Option<Vec<BreadcrumbText>> {
        let title = self.title(cx);
        Some(vec![
            BreadcrumbText {
                text: "Agent".into(),
                highlights: None,
                font: None,
            },
            BreadcrumbText {
                text: title,
                highlights: None,
                font: None,
            },
        ])
    }
}
```

### 5.2 Serialization for Persistence

**File**: `crates/agent_ui/src/agent_chat_view.rs`

```rust
impl SerializableItem for AgentChatView {
    fn serialized_item_kind() -> &'static str {
        "AgentChatView"
    }

    fn cleanup(
        workspace_id: WorkspaceId,
        alive_items: Vec<ItemId>,
        cx: &mut App,
    ) -> Task<Result<()>> {
        // Clean up any orphaned agent chat state
        Task::ready(Ok(()))
    }

    fn deserialize(
        project: Entity<Project>,
        workspace: WeakEntity<Workspace>,
        workspace_id: WorkspaceId,
        item_id: ItemId,
        cx: &mut App,
    ) -> Task<Result<Entity<Self>>> {
        // Restore agent chat from serialized state
        let prompt_builder = ...; // Get from global state
        Self::load(workspace, prompt_builder, cx.to_async())
    }

    fn serialize(
        &self,
        workspace_id: WorkspaceId,
        item_id: ItemId,
        cx: &App,
    ) -> Option<Task<Result<()>>> {
        // Serialize current state (active thread, view mode, etc.)
        let state = self.content.read(cx).serialized_state();
        Some(cx.background_spawn(async move {
            // Save to database
            Ok(())
        }))
    }

    fn should_serialize(&self, _event: &Self::Event) -> bool {
        true
    }
}

// Register with workspace
pub fn register_serializable_item(cx: &mut App) {
    workspace::register_serializable_item::<AgentChatView>(cx);
}
```

---

## 6. Actions and Keybindings

### 6.1 New Actions

**File**: `crates/zed_actions/src/agent.rs`

```rust
// Open agent chat in center pane
actions!(agent, [
    OpenAgentChat,         // Open as tab
    OpenAgentChatSplit,    // Open as split with current item
    ToggleAgentChat,       // Toggle visibility
    FocusAgentChat,        // Focus if open, else open
]);
```

### 6.2 Default Keybindings

**File**: `assets/keymaps/default-macos.json`

```json
{
    "bindings": {
        // Primary keybinding for agent chat
        "cmd-shift-a": "agent::OpenAgentChat",

        // Alternative: toggle focus (similar to current panel behavior)
        "cmd-?": "agent::FocusAgentChat",

        // Open as split
        "cmd-shift-\\": "agent::OpenAgentChatSplit"
    }
}
```

### 6.3 Action Handlers

**File**: `crates/zed/src/zed.rs`

```rust
fn initialize_workspace(...) {
    // ... existing code ...

    cx.observe_new(|workspace: &mut Workspace, window, cx| {
        workspace
            .register_action(|workspace, _: &OpenAgentChat, window, cx| {
                AgentChatView::open(workspace, get_prompt_builder(cx), window, cx);
            })
            .register_action(|workspace, _: &OpenAgentChatSplit, window, cx| {
                AgentChatView::open_as_split(workspace, get_prompt_builder(cx), window, cx);
            })
            .register_action(|workspace, _: &FocusAgentChat, window, cx| {
                // If agent chat exists, focus it; otherwise open it
                if let Some(chat) = workspace.items_of_type::<AgentChatView>(cx).next() {
                    workspace.activate_item(&chat, true, true, window, cx);
                } else {
                    AgentChatView::open(workspace, get_prompt_builder(cx), window, cx);
                }
            })
            .register_action(|workspace, _: &ToggleAgentChat, window, cx| {
                // Toggle: close if focused, open if not
                if let Some(chat) = workspace.active_item_as::<AgentChatView>(cx) {
                    workspace.close_item(chat.item_id(), CloseIntent::Close, window, cx);
                } else {
                    AgentChatView::open(workspace, get_prompt_builder(cx), window, cx);
                }
            });
    });
}
```

---

## 7. Startup Behavior

### 7.1 Default Startup: Open Agent Chat

**File**: `crates/zed/src/main.rs`

In the workspace restoration logic:

```rust
async fn restore_or_create_workspace(...) {
    // ... existing restoration logic ...

    // After workspace is created/restored, ensure agent chat is open
    workspace.update_in(&mut cx, |workspace, window, cx| {
        let settings = AgentChatSettings::get_global(cx);

        if settings.open_on_startup {
            // Check if there's already an agent chat item
            let has_agent_chat = workspace.items_of_type::<AgentChatView>(cx).next().is_some();

            if !has_agent_chat {
                AgentChatView::open(workspace, get_prompt_builder(cx), window, cx);
            }
        }
    })?;
}
```

### 7.2 New Workspace Behavior

**File**: `crates/workspace/src/workspace.rs`

When creating a new workspace, open agent chat:

```rust
impl Workspace {
    pub fn new(...) -> Self {
        // ... existing initialization ...

        // Schedule agent chat opening for new workspaces
        if is_new_workspace && AgentChatSettings::get_global(cx).open_on_startup {
            cx.defer(|workspace, window, cx| {
                AgentChatView::open(workspace, get_prompt_builder(cx), window, cx);
            });
        }
    }
}
```

### 7.3 Settings for Startup

**File**: `crates/agent_settings/src/agent_settings.rs`

```rust
#[derive(Clone, Debug, RegisterSetting)]
pub struct AgentChatSettings {
    /// Whether to open agent chat when creating a new workspace
    pub open_on_startup: bool,

    /// Whether to open agent chat as split when opening files
    pub open_files_as_split: bool,

    /// Default width ratio when split (0.0 to 1.0)
    pub split_ratio: f32,

    /// Whether to show session history sidebar
    pub show_history_sidebar: bool,
}

impl Default for AgentChatSettings {
    fn default() -> Self {
        Self {
            open_on_startup: true,
            open_files_as_split: true,
            split_ratio: 0.6,  // Agent takes 60%
            show_history_sidebar: true,
        }
    }
}
```

---

## 8. File Opening Behavior

### 8.1 Files Open as Splits

When agent chat is focused and user opens a file, open as split:

**File**: `crates/workspace/src/workspace.rs`

```rust
impl Workspace {
    pub fn open_path(
        &mut self,
        path: ProjectPath,
        ...,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<...> {
        // Check if agent chat is active and split mode is enabled
        let should_split = self.active_item_as::<AgentChatView>(cx).is_some()
            && AgentChatSettings::get_global(cx).open_files_as_split;

        if should_split {
            // Open as split to the right of agent chat
            self.open_path_as_split(path, SplitDirection::Right, window, cx)
        } else {
            // Normal open behavior
            self.open_path_internal(path, window, cx)
        }
    }

    fn open_path_as_split(
        &mut self,
        path: ProjectPath,
        direction: SplitDirection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<...> {
        let open_task = self.open_path_internal(path, window, cx);

        cx.spawn_in(window, async move |workspace, cx| {
            let item = open_task.await?;

            workspace.update_in(cx, |workspace, window, cx| {
                // Create split pane
                let active_pane = workspace.active_pane();

                // Find or create the right pane
                let target_pane = workspace.find_or_create_split_pane(
                    &active_pane,
                    direction,
                    window,
                    cx,
                );

                // Move item to target pane
                workspace.move_item_to_pane(item, &target_pane, window, cx);
            })?;

            Ok(item)
        })
    }
}
```

### 8.2 Agent Chat Handles File Opens

**File**: `crates/agent_ui/src/agent_chat_view.rs`

When thread references a file, clicking opens as split:

```rust
impl AgentChatView {
    fn handle_file_click(&self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(workspace) = self.workspace.upgrade() {
            workspace.update(cx, |workspace, cx| {
                let project_path = ProjectPath { ... };
                workspace.open_path_as_split(
                    project_path,
                    SplitDirection::Right,
                    window,
                    cx,
                ).detach_and_log_err(cx);
            });
        }
    }
}
```

---

## 9. Settings

### 9.1 New Settings Schema

**File**: `crates/settings_content/src/agent.rs`

```rust
/// Settings for the agent chat center view
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct AgentChatSettingsContent {
    /// Whether to open agent chat when starting a new workspace.
    ///
    /// Default: true
    pub open_on_startup: Option<bool>,

    /// Whether files opened while agent chat is focused should
    /// open as splits instead of replacing the current tab.
    ///
    /// Default: true
    pub open_files_as_split: Option<bool>,

    /// The ratio of width given to agent chat when splitting.
    /// 0.6 means agent chat takes 60% of the width.
    ///
    /// Default: 0.6
    pub split_ratio: Option<f32>,

    /// Whether to show the session history sidebar in agent chat.
    ///
    /// Default: true
    pub show_history_sidebar: Option<bool>,

    /// Whether to pin the agent chat tab (prevent accidental closing).
    ///
    /// Default: false
    pub pin_agent_tab: Option<bool>,
}
```

### 9.2 Default Settings

**File**: `assets/settings/default.json`

```json
{
    "agent_chat": {
        "open_on_startup": true,
        "open_files_as_split": true,
        "split_ratio": 0.6,
        "show_history_sidebar": true,
        "pin_agent_tab": false
    }
}
```

---

## 10. Serialization and Persistence

### 10.1 What to Serialize

When workspace is saved:
- Whether agent chat was open
- Active thread ID
- Current view (thread, history, config)
- Split positions
- History sidebar state

### 10.2 SerializedAgentChatView

**File**: `crates/agent_ui/src/agent_chat_view.rs`

```rust
#[derive(Serialize, Deserialize)]
struct SerializedAgentChatView {
    /// Current thread ID (if viewing a thread)
    active_thread_id: Option<String>,

    /// Which view is active
    active_view: SerializedActiveView,

    /// Whether history sidebar is visible
    history_sidebar_visible: bool,

    /// Selected agent type
    selected_agent: AgentType,
}

#[derive(Serialize, Deserialize)]
enum SerializedActiveView {
    Thread { thread_id: String },
    History { kind: HistoryKind },
    Configuration,
}
```

### 10.3 Workspace Item Persistence

**File**: `crates/workspace/src/persistence.rs`

Agent chat items are serialized with the workspace:

```rust
// When saving workspace
let agent_chat_items: Vec<SerializedAgentChatView> = workspace
    .items_of_type::<AgentChatView>(cx)
    .map(|item| item.read(cx).to_serialized())
    .collect();

// When restoring workspace
for serialized in agent_chat_items {
    AgentChatView::from_serialized(serialized, workspace, window, cx);
}
```

---

## 11. Navigation and Focus

### 11.1 Focus Behavior

```rust
impl AgentChatView {
    /// Focus the message input
    pub fn focus_message_input(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.content.update(cx, |content, cx| {
            if let Some(thread_view) = content.active_thread_view() {
                thread_view.update(cx, |view, cx| {
                    view.focus_message_editor(window, cx);
                });
            }
        });
    }

    /// Focus the history sidebar
    pub fn focus_history(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.content.update(cx, |content, cx| {
            content.focus_history(window, cx);
        });
    }
}
```

### 11.2 Keyboard Navigation

**File**: `assets/keymaps/default-macos.json`

```json
{
    "context": "AgentChatView",
    "bindings": {
        // Navigation within agent chat
        "cmd-1": "agent::FocusHistory",
        "cmd-2": "agent::FocusThread",
        "cmd-n": "agent::NewThread",
        "escape": "agent::FocusMessageInput",

        // History navigation
        "cmd-[": "agent::GoBack",
        "cmd-]": "agent::GoForward",

        // Toggle sidebar
        "cmd-b": "agent::ToggleHistorySidebar"
    }
}
```

---

## 12. Relationship with AgentPanel

### 12.1 Coexistence Strategy

Both `AgentChatView` (center) and `AgentPanel` (dock) can exist:

| Feature | AgentChatView | AgentPanel |
|---------|---------------|------------|
| Location | Center pane | Dock (left/right) |
| Primary use | Main workspace | Quick access |
| Session history | Built-in sidebar | Built-in |
| File splits | Native pane splits | Manual |
| Default | Yes (new behavior) | No (legacy) |

### 12.2 Shared State

Both use the same `AgentChatContent`, so:
- Same thread store
- Same history
- Same configuration
- Can show same thread simultaneously

### 12.3 User Choice

**File**: `assets/settings/default.json`

```json
{
    "agent": {
        // Which mode to use
        "ui_mode": "center",  // "center" | "panel" | "both"

        // Legacy panel settings (used when ui_mode is "panel" or "both")
        "dock": "right",
        "default_width": 400
    },
    "agent_chat": {
        // Center view settings (used when ui_mode is "center" or "both")
        "open_on_startup": true,
        "split_ratio": 0.6
    }
}
```

---

## 13. Implementation Phases

### Phase 1: Extract AgentChatContent (2-3 days)

1. Create `agent_chat_content.rs`
2. Move state from `AgentPanel` to `AgentChatContent`
3. Move methods from `AgentPanel` to `AgentChatContent`
4. Refactor `AgentPanel` to use `AgentChatContent`
5. Ensure all existing tests pass

**Files Changed:**
- `crates/agent_ui/src/agent_chat_content.rs` (NEW)
- `crates/agent_ui/src/agent_panel.rs` (MODIFIED)
- `crates/agent_ui/src/mod.rs` (MODIFIED)

### Phase 2: Create AgentChatView (2-3 days)

1. Create `agent_chat_view.rs`
2. Implement basic `AgentChatView` struct
3. Implement `Render` trait
4. Implement `Focusable` trait
5. Implement `EventEmitter` trait
6. Add basic tests

**Files Changed:**
- `crates/agent_ui/src/agent_chat_view.rs` (NEW)
- `crates/agent_ui/src/mod.rs` (MODIFIED)

### Phase 3: Implement Item Trait (2 days)

1. Implement `Item` trait for `AgentChatView`
2. Implement tab content rendering
3. Implement navigation
4. Implement splitting
5. Add item-specific tests

**Files Changed:**
- `crates/agent_ui/src/agent_chat_view.rs` (MODIFIED)

### Phase 4: Actions and Keybindings (1 day)

1. Add new actions to `zed_actions`
2. Register action handlers in workspace
3. Add default keybindings
4. Test keybindings

**Files Changed:**
- `crates/zed_actions/src/agent.rs` (MODIFIED)
- `crates/zed/src/zed.rs` (MODIFIED)
- `assets/keymaps/default-macos.json` (MODIFIED)
- `assets/keymaps/default-linux.json` (MODIFIED)

### Phase 5: Startup and File Opening (2 days)

1. Add startup settings
2. Implement open-on-startup behavior
3. Implement file-opens-as-split behavior
4. Test various startup scenarios

**Files Changed:**
- `crates/agent_settings/src/agent_settings.rs` (MODIFIED)
- `crates/settings_content/src/agent.rs` (MODIFIED)
- `crates/zed/src/main.rs` (MODIFIED)
- `crates/workspace/src/workspace.rs` (MODIFIED)
- `assets/settings/default.json` (MODIFIED)

### Phase 6: Serialization (1-2 days)

1. Implement `SerializableItem` for `AgentChatView`
2. Register with workspace persistence
3. Test save/restore cycles

**Files Changed:**
- `crates/agent_ui/src/agent_chat_view.rs` (MODIFIED)
- `crates/workspace/src/persistence.rs` (MODIFIED)

### Phase 7: Polish and Testing (2 days)

1. UI polish (animations, transitions)
2. Edge case handling
3. Performance optimization
4. Documentation
5. Full test suite

**Files Changed:**
- Various UI tweaks
- Test files

### Total Estimated Time: 12-15 days

---

## 14. Testing Strategy

### 14.1 Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    async fn test_agent_chat_view_creation(cx: &mut TestAppContext) {
        // Test basic creation
    }

    #[gpui::test]
    async fn test_agent_chat_view_tab_content(cx: &mut TestAppContext) {
        // Test tab rendering
    }

    #[gpui::test]
    async fn test_agent_chat_view_splitting(cx: &mut TestAppContext) {
        // Test can_split and clone_on_split
    }

    #[gpui::test]
    async fn test_file_opens_as_split(cx: &mut TestAppContext) {
        // Test that files open as splits when agent chat is focused
    }

    #[gpui::test]
    async fn test_serialization_roundtrip(cx: &mut TestAppContext) {
        // Test save/restore
    }
}
```

### 14.2 Integration Tests

```rust
#[gpui::test]
async fn test_startup_opens_agent_chat(cx: &mut TestAppContext) {
    // Test that new workspace opens with agent chat
}

#[gpui::test]
async fn test_agent_chat_and_panel_share_content(cx: &mut TestAppContext) {
    // Test that both views show same thread
}
```

### 14.3 Manual Testing Checklist

- [ ] New workspace opens with agent chat
- [ ] Can create new threads
- [ ] Can switch between threads in history
- [ ] Files open as splits to the right
- [ ] Tab shows thread title
- [ ] Tab shows unsent message indicator
- [ ] Can split agent chat with another agent chat
- [ ] Workspace save/restore works
- [ ] Keybindings work
- [ ] Settings changes take effect
- [ ] Focus management works correctly
- [ ] Performance is acceptable with large threads

---

## 15. File Changes Summary

### New Files

| File | Description |
|------|-------------|
| `crates/agent_ui/src/agent_chat_content.rs` | Shared content logic |
| `crates/agent_ui/src/agent_chat_view.rs` | Center pane item |

### Modified Files

| File | Changes |
|------|---------|
| `crates/agent_ui/src/agent_panel.rs` | Refactor to use AgentChatContent |
| `crates/agent_ui/src/mod.rs` | Export new modules |
| `crates/agent_ui/src/agent_ui.rs` | Register AgentChatView |
| `crates/agent_settings/src/agent_settings.rs` | Add new settings |
| `crates/settings_content/src/agent.rs` | Add settings schema |
| `crates/zed_actions/src/agent.rs` | Add new actions |
| `crates/zed/src/zed.rs` | Register action handlers |
| `crates/zed/src/main.rs` | Startup behavior |
| `crates/workspace/src/workspace.rs` | File split logic |
| `assets/settings/default.json` | Default settings |
| `assets/keymaps/default-macos.json` | Keybindings |
| `assets/keymaps/default-linux.json` | Keybindings |

### Lines of Code Estimate

| Component | Lines |
|-----------|-------|
| AgentChatContent | ~800 (extracted from AgentPanel) |
| AgentChatView | ~500 |
| Item trait impl | ~200 |
| Actions/Keybindings | ~100 |
| Settings | ~100 |
| Tests | ~300 |
| **Total new code** | ~2000 |
