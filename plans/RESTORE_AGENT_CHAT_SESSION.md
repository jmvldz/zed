# Restore Previously Open Agent Chat Session

## Summary

Implemented functionality to restore the previously open agent chat thread when reopening a workspace, instead of creating a new empty thread.

## Problem

When closing and reopening Zed, agent chat tabs were restored but always showed a new empty thread instead of the thread that was previously open. The `AgentChatView` was being serialized/deserialized but only persisted the `selected_agent` type, not the active thread's session ID.

## Solution

### Files Modified

1. **`crates/agent_ui/src/agent_chat_view.rs`**
2. **`crates/agent_ui/src/agent_chat_content.rs`**
3. **`crates/agent_ui/src/acp/thread_view.rs`**

### Changes

#### 1. Added `session_id` to Serialized State

**`agent_chat_view.rs`**

Added `session_id` field to `SerializedAgentChatView`:
```rust
#[derive(Debug, Serialize, Deserialize)]
struct SerializedAgentChatView {
    selected_agent: Option<crate::agent_chat_content::AgentType>,
    #[serde(default)]
    session_id: Option<String>,  // NEW
}
```

#### 2. Database Migration

**`agent_chat_view.rs`** - Added migration to add `session_id` column:
```rust
const MIGRATIONS: &[&str] = &[
    sql!(CREATE TABLE agent_chat_views(...) STRICT;),
    sql!(ALTER TABLE agent_chat_views ADD COLUMN session_id TEXT;),  // NEW
];
```

Updated `save_state` and `get_state` to handle the new column.

#### 3. Added `active_session_id()` Helper

**`agent_chat_content.rs`**

Returns the session ID of the active thread:
```rust
pub fn active_session_id(&self, cx: &App) -> Option<String> {
    match &self.active_view {
        ActiveView::ExternalAgentThread { thread_view } => {
            thread_view.read(cx).session_id(cx).map(|id| id.to_string())
        }
        _ => None,
    }
}
```

#### 4. Added `session_id()` Method to AcpThreadView

**`thread_view.rs`**

Gets session ID from thread (if Ready) or from `resume_thread_metadata`:
```rust
pub fn session_id(&self, cx: &App) -> Option<acp::SessionId> {
    if let Some(thread) = self.thread() {
        return Some(thread.read(cx).session_id().clone());
    }
    self.resume_thread_metadata.as_ref().map(|m| m.session_id.clone())
}
```

#### 5. Store Session ID When Thread Becomes Ready

**`thread_view.rs`**

When thread transitions from `Loading` to `Ready`, store the session ID in `resume_thread_metadata` so it's available for serialization:
```rust
// In the Ready state transition code:
if this.resume_thread_metadata.is_none() {
    this.resume_thread_metadata = Some(AgentSessionInfo::new(session_id.clone()));
}
```

This was necessary because:
- Thread starts in `Loading` state
- Serialization can happen while still `Loading`
- `thread()` only returns a value in `Ready` state
- By storing in `resume_thread_metadata`, the session ID is available even during `Loading`

#### 6. Updated `restore_agent()` to Handle Different Agent Types

**`agent_chat_content.rs`**

Different agent types require different restoration approaches:

- **NativeAgent**: Look up thread from `ThreadStore` by session ID, pass as `summarize_thread`
- **External Agents** (ClaudeCode, Gemini, Codex, Custom): Pass session ID as `resume_thread` for ACP `session/load`

```rust
pub fn restore_agent(
    &mut self,
    agent_type: AgentType,
    session_id: Option<String>,
    window: &mut Window,
    cx: &mut Context<Self>,
) {
    self.selected_agent = agent_type.clone();

    if let Some(session_id) = session_id {
        match &agent_type {
            AgentType::NativeAgent => {
                // Look up from ThreadStore, pass as summarize_thread
                let thread = self.thread_store.read(cx).thread_from_session_id(&session_id);
                if let Some(thread) = thread {
                    self.external_thread(Some(ExternalAgent::NativeAgent), None, Some(session_info), window, cx);
                    return;
                }
            }
            AgentType::Gemini | AgentType::ClaudeCode | AgentType::Codex | AgentType::Custom { .. } => {
                // Pass as resume_thread for ACP session/load
                let resume_info = AgentSessionInfo::new(acp::SessionId::new(session_id));
                self.external_thread(Some(agent), Some(resume_info), None, window, cx);
                return;
            }
            AgentType::TextThread => {}
        }
    }

    self.new_agent_thread(agent_type, window, cx);
}
```

#### 7. Updated `serialize()` and `deserialize()`

**`agent_chat_view.rs`**

- `serialize()`: Now captures `session_id` from `active_session_id()`
- `deserialize()`: Now passes `session_id` to `restore_agent()`

## Key Insights

1. **AcpThreadView State Machine**: The thread goes through states (`Loading` -> `Ready`), and `thread()` only returns a value in `Ready` state. Serialization can happen at any time, so we needed a way to access the session ID even when not Ready.

2. **NativeAgent vs External Agents**: They use different restoration mechanisms:
   - NativeAgent: Threads stored locally in `ThreadStore`, restored via `summarize_thread` parameter
   - External Agents: Use ACP protocol's `session/load` method via `resume_thread` parameter

3. **Timing of Serialization**: Serialization happens on various workspace events, not specifically when the thread becomes Ready. We needed to store the session ID when the thread becomes Ready so subsequent serializations would have it.

## Debug Logging (to be removed)

Added extensive debug logging to trace the flow. These should be removed before merging:
- `agent_chat_view.rs`: serialize/deserialize logging
- `agent_chat_content.rs`: active_session_id and restore_agent logging
- `thread_view.rs`: session_id method and Ready state transition logging

## Testing

1. Open Zed, start a new agent thread, send a message
2. Close and reopen Zed
3. Verify the same thread is restored (check title, message history)
4. Verify creating a new thread still works after restoration
5. Test with different agent types (NativeAgent, ClaudeCode, etc.)

## Status

Implementation complete, needs testing and removal of debug logging before merge.
