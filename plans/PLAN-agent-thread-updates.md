# Implementation Plan: Agent Thread UI Updates

## Overview

This plan covers three main changes to how agent threads are treated in the tab bar:
1. **Dynamic tab icons** - Tab shows the icon of the currently selected agent
2. **Auto-generated titles** - Use LLM to summarize and create short titles for threads
3. **Move menu items to tab bar** - Move "New Thread", "History", and "Settings" into the tab bar's "+" menu

---

## 1. Dynamic Tab Icons

**Goal**: The tab should display the icon of the currently selected agent (Claude, Gemini, OpenAI, Zed Agent, etc.) instead of always showing `ZedAssistant`.

### Current Implementation

**File**: `crates/agent_ui/src/agent_chat_view.rs:245-247`
```rust
fn tab_icon(&self, _window: &Window, _cx: &App) -> Option<Icon> {
    Some(Icon::new(IconName::ZedAssistant).color(Color::Muted))
}
```

### Changes Required

**File**: `crates/agent_ui/src/agent_chat_view.rs`

1. Modify `tab_icon()` to read the selected agent from `self.content`:

```rust
fn tab_icon(&self, _window: &Window, cx: &App) -> Option<Icon> {
    let content = self.content.read(cx);
    let agent_type = &content.selected_agent;

    // Check for custom agent icon first
    if let AgentType::Custom { name } = agent_type {
        let agent_server_store = content.project.read(cx).agent_server_store().clone();
        if let Some(icon_path) = agent_server_store.read(cx).agent_icon(
            &project::ExternalAgentServerName(name.clone())
        ) {
            return Some(Icon::from_external_svg(icon_path).color(Color::Muted));
        }
    }

    // Fall back to built-in icon
    agent_type.icon()
        .map(|icon_name| Icon::new(icon_name).color(Color::Muted))
        .or_else(|| Some(Icon::new(IconName::ZedAssistant).color(Color::Muted)))
}
```

2. The `AgentType::icon()` method already exists at `crates/agent_ui/src/agent_chat_content.rs:143-152`:
```rust
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
```

3. Ensure `cx.notify()` is called when the selected agent changes so the tab icon updates. Check `AgentChatContent::set_selected_agent()` or similar methods.

### Files to Modify
- `crates/agent_ui/src/agent_chat_view.rs` - Update `tab_icon()` implementation

---

## 2. Auto-Generated Titles via LLM Summarization

**Goal**: Automatically generate a short title for the thread using an LLM to summarize the conversation.

### Current State: Native vs External Agents

**Native threads work** - Title generation exists in `crates/agent/src/thread.rs:2085-2140` and auto-triggers after the first turn completes.

**External agents DON'T work** - `AcpThread` has no title generation capability. There's a TODO at `crates/agent_ui/src/acp/thread_view.rs:6779` confirming this is a known limitation.

### Architecture Differences

| Feature | Native Thread (`Thread`) | External Agent Thread (`AcpThread`) |
|---------|--------------------------|-------------------------------------|
| File | `crates/agent/src/thread.rs` | `crates/acp_thread/src/acp_thread.rs` |
| Title field | `title: Option<SharedString>` | `title: SharedString` |
| Summarization model | `summarization_model: Option<Arc<dyn LanguageModel>>` | **None** |
| `generate_title()` | Yes (line 2085) | **No** |
| Auto-triggers | After first turn (line 1669-1673) | **Never** |
| "Regenerate Title" menu | Works via `as_native_thread()` | Returns `None`, does nothing |

### Changes Required for External Agents

**1. Add title generation to `AcpThread`**

**File**: `crates/acp_thread/src/acp_thread.rs`

Add new fields and methods:

```rust
pub struct AcpThread {
    // ... existing fields ...

    // NEW: For title generation
    pending_title_generation: Option<Task<()>>,
}

impl AcpThread {
    /// Generate a title using any available LLM
    pub fn generate_title(&mut self, cx: &mut Context<Self>) {
        if self.pending_title_generation.is_some() {
            return;
        }

        // Get messages for summarization
        let messages = self.entries_for_summarization();
        if messages.is_empty() {
            return;
        }

        // Use LanguageModelRegistry to get any available model
        let model = LanguageModelRegistry::global(cx)
            .read(cx)
            .active_model()
            .or_else(|| /* fallback to any available model */);

        let Some(model) = model else { return };

        self.pending_title_generation = Some(cx.spawn(|this, mut cx| async move {
            // Stream completion with SUMMARIZE_THREAD_PROMPT
            // Extract first line as title
            // Call this.update(cx, |this, cx| this.set_title(title, cx))
        }));
    }

    pub fn is_generating_title(&self) -> bool {
        self.pending_title_generation.is_some()
    }

    fn entries_for_summarization(&self) -> Vec<LanguageModelRequestMessage> {
        // Convert thread entries to LLM request messages
        // Similar to Thread::messages_for_summarization()
    }
}
```

**2. Trigger title generation after turns complete**

**File**: `crates/acp_thread/src/acp_thread.rs`

In the code that handles turn completion (when status becomes idle), add:

```rust
// After a turn completes
if self.title.is_empty() && self.pending_title_generation.is_none() {
    self.generate_title(cx);
}
```

Look for where `AcpThreadEvent::StatusChanged` is emitted or where the thread transitions to idle state.

**3. Update "Regenerate Thread Title" menu**

**File**: `crates/agent_ui/src/agent_chat_content.rs:1490-1505`

Current code only handles native threads:
```rust
if let Some(native_thread) = thread_view.read(cx).as_native_thread(cx) {
    native_thread.update(cx, |thread, cx| thread.generate_title(cx));
}
```

Change to also handle ACP threads:
```rust
if let Some(native_thread) = thread_view.read(cx).as_native_thread(cx) {
    native_thread.update(cx, |thread, cx| thread.generate_title(cx));
} else if let Some(acp_thread) = thread_view.read(cx).thread() {
    acp_thread.update(cx, |thread, cx| thread.generate_title(cx));
}
```

**4. Make title editable for ACP threads**

**File**: `crates/agent_ui/src/acp/thread_view.rs:784-799`

Currently, title editor is only created if `can_set_title()` returns true, which requires `connection.set_title()` to return `Some`. For local title changes (not synced to external agent), we should allow editing regardless:

```rust
// Always allow title editing for generated titles
let title_editor = Some(cx.new(|cx| {
    let mut editor = Editor::single_line(window, cx);
    editor.set_text(thread.read(cx).title(), window, cx);
    editor
}));
```

**5. Persist titles for ACP threads**

Ensure generated titles are saved to the session storage. Check `AgentSessionInfo` in `crates/acp_thread/src/connection.rs:208-215` - titles should be persisted when updated.

### Files to Modify

| File | Changes |
|------|---------|
| `crates/acp_thread/src/acp_thread.rs` | Add `generate_title()`, `is_generating_title()`, `pending_title_generation` field |
| `crates/agent_ui/src/agent_chat_content.rs` | Update "Regenerate Title" menu to handle ACP threads |
| `crates/agent_ui/src/acp/thread_view.rs` | Allow title editor for ACP threads, remove TODO at line 6779 |

### Native Thread Verification (Lower Priority)

For native threads, the existing implementation should work. Verify:
- `Thread::generate_title()` is called after first turn (line 1669-1673)
- `ThreadEvent::TitleUpdated` propagates to `AgentChatContentEvent::TitleChanged`
- `AgentChatView` emits `ItemEvent::UpdateTab` on title change (line 99)

---

## 3. Move Menu Items to Tab Bar

**Goal**:
- Move "New Thread" options into the tab bar's "+" popover menu
- Move "History" and "Settings" into a **separate menu** next to the split pane button

### Current Locations

The current toolbar buttons are in `crates/agent_ui/src/agent_chat_content.rs:1564-1602`:
```rust
pub fn render_toolbar(&mut self, ...) -> gpui::AnyElement {
    let right_section = h_flex()
        .child(self.render_new_thread_menu(window, cx))      // "+" button
        .child(self.render_recent_entries_menu(...))          // History
        .child(self.render_panel_options_menu(window, cx));   // "..." settings
}
```

### Target Layout

```
Tab Bar:  [tabs...] [+] [⋯] [⊞] [↗]
                     │   │   │   └── Maximize
                     │   │   └────── Split pane
                     │   └────────── NEW: Agent menu (History, Settings)
                     └────────────── New Thread options added here
```

### Implementation

Use `Pane::set_render_tab_bar_buttons()` to provide custom buttons when an agent chat is active.

**File**: `crates/workspace/src/pane.rs:4064-4158`

The pane allows setting custom tab bar button renderers:
```rust
pub fn set_render_tab_bar_buttons(
    &mut self,
    cx: &mut Context<Self>,
    render: impl Fn(&mut Pane, &mut Window, &mut Context<Pane>) -> (Option<AnyElement>, Option<AnyElement>) + 'static,
)
```

Returns `(left_buttons, right_buttons)` - we'll add our buttons to the right side.

### Changes Required

**1. Add custom tab bar buttons in `AgentChatView`**

**File**: `crates/agent_ui/src/agent_chat_view.rs`

```rust
fn install_tab_bar_buttons(&self, pane: &Entity<Pane>, cx: &mut Context<Self>) {
    let content = self.content.clone();
    pane.update(cx, |pane, cx| {
        pane.set_render_tab_bar_buttons(cx, move |pane, window, cx| {
            Self::render_agent_tab_bar_buttons(&content, pane, window, cx)
        });
    });
}

fn render_agent_tab_bar_buttons(
    content: &Entity<AgentChatContent>,
    pane: &mut Pane,
    window: &mut Window,
    cx: &mut Context<Pane>,
) -> (Option<AnyElement>, Option<AnyElement>) {
    let left = None;

    let right = h_flex()
        .gap_1()
        // "+" menu - New Thread options
        .child(
            PopoverMenu::new("new-thread-menu")
                .trigger_with_tooltip(
                    IconButton::new("plus", IconName::Plus).icon_size(IconSize::Small),
                    Tooltip::text("New..."),
                )
                .anchor(Corner::TopRight)
                .menu(move |window, cx| {
                    Some(ContextMenu::build(window, cx, |menu, _, _| {
                        menu
                            // Agent thread options
                            .header("New Thread")
                            .action("Zed Agent", NewThread::agent().boxed_clone())
                            .action("Text Thread", NewTextThread.boxed_clone())
                            .separator()
                            .header("External Agents")
                            .action("Claude Code", NewThread::claude_code().boxed_clone())
                            // ... other external agents from AgentServerStore
                            .separator()
                            // Standard pane items
                            .action("New File", NewFile.boxed_clone())
                            .action("Open File", ToggleFileFinder::default().boxed_clone())
                            .separator()
                            .action("New Terminal", NewTerminal::default().boxed_clone())
                    }))
                })
        )
        // "⋯" menu - History and Settings (SEPARATE from "+")
        .child(
            PopoverMenu::new("agent-options-menu")
                .trigger_with_tooltip(
                    IconButton::new("ellipsis", IconName::Ellipsis).icon_size(IconSize::Small),
                    Tooltip::text("Agent Options"),
                )
                .anchor(Corner::TopRight)
                .menu(move |window, cx| {
                    Some(ContextMenu::build(window, cx, |menu, _, _| {
                        menu
                            .action("History", OpenHistory.boxed_clone())
                            .action("Settings", zed_actions::agent::OpenSettings.boxed_clone())
                    }))
                })
        )
        .into_any_element();

    (left, Some(right))
}
```

**2. Call `install_tab_bar_buttons` when agent chat is added to pane**

Implement `Item::added_to_pane()`:
```rust
fn added_to_pane(
    &mut self,
    pane: Entity<Pane>,
    _workspace: Option<Entity<Workspace>>,
    _window: &mut Window,
    cx: &mut Context<Self>,
) {
    self.install_tab_bar_buttons(&pane, cx);
}
```

**3. Remove toolbar buttons from `AgentChatContent::render_toolbar()`**

**File**: `crates/agent_ui/src/agent_chat_content.rs:1564-1602`

Remove the "+" (new thread), history menu, and "..." (options) buttons from the toolbar since they're now in the tab bar.

**4. Ensure actions dispatch correctly**

The actions `NewThread`, `OpenHistory`, `OpenSettings` need to be dispatchable from the pane level. Verify action registration in `agent_ui.rs`.

### Files to Modify

| File | Changes |
|------|---------|
| `crates/agent_ui/src/agent_chat_view.rs` | Add `install_tab_bar_buttons()`, `render_agent_tab_bar_buttons()`, implement `added_to_pane()` |
| `crates/agent_ui/src/agent_chat_content.rs` | Remove toolbar buttons that moved to tab bar |
| `crates/workspace/src/pane.rs` | May need to verify button ordering (new buttons should appear before split/maximize) |

---

## Summary of Files to Modify

| Feature | Files |
|---------|-------|
| Dynamic Tab Icons | `crates/agent_ui/src/agent_chat_view.rs` |
| Auto-Generated Titles (Native) | `crates/agent/src/thread.rs` (verify existing) |
| Auto-Generated Titles (External) | `crates/acp_thread/src/acp_thread.rs`, `crates/agent_ui/src/agent_chat_content.rs`, `crates/agent_ui/src/acp/thread_view.rs` |
| Tab Bar Menu | `crates/agent_ui/src/agent_chat_view.rs`, `crates/agent_ui/src/agent_chat_content.rs` |

## Implementation Order

1. **Tab Icons** (simplest, isolated change)
2. **Title Generation for External Agents** (most significant new feature)
   - Add `generate_title()` to `AcpThread`
   - Trigger after turn completion
   - Update menu to handle both thread types
3. **Tab Bar Menu** (requires coordination between pane and agent view)

## Testing Considerations

1. **Tab Icons**: Open threads with different agent types, verify icons update correctly
2. **Titles**: Start new threads, send messages, verify titles generate automatically
3. **Tab Menu**: Verify all menu items work, keyboard shortcuts are shown, actions dispatch correctly
4. **Edge Cases**:
   - Custom external agents with SVG icons
   - Threads without titles (should show "New Thread" or similar)
   - Multiple agent tabs open simultaneously
