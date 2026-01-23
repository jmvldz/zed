# Multi-Worktree Panel Implementation Summary

## Overview

Implemented a per-window panel for displaying and switching between git worktrees belonging to the current repository. This enables users to quickly navigate between different worktrees without opening separate windows.

## What Was Built

### Phase 1: WorktreeRegistry Foundation

**File:** `crates/workspace/src/worktree_registry.rs` (new)

Created the core data layer for tracking worktrees:

- `WorktreeSlotId` - Stable identifier derived from worktree path
- `WorktreeEntry` - Contains slot_id, worktree_path, branch_name, last_accessed, agent_chat_count
- `WorktreeSlot` - Manages slot state (Active/Cached/Unloaded) and cached Project references
- `WorktreeRegistry` - Window-scoped entity that:
  - Tracks all worktrees for the current repo
  - Scans git worktrees via Repository API
  - Manages active slot selection
  - Emits events on changes (ActiveSlotChanged, WorktreeAdded, WorktreeRemoved, WorktreesScanned)
  - Auto-cleans cached projects after timeout (5 minutes)

**Integration:** Added `worktree_registry: Option<Entity<WorktreeRegistry>>` to Workspace struct with automatic initialization during workspace creation.

### Phase 2: WorktreesPanel UI

**Files:** `crates/worktrees_panel/` (new crate)

Created the panel UI:

- Shows repository name/path in header
- Lists all worktrees with:
  - Branch name
  - Last accessed time (formatted as "just now", "5m ago", "2h ago", etc.)
  - Agent chat count badge
- Active worktree highlighted with bold text
- Click to open/switch to worktree
- Implements Panel trait for dock integration

**Settings:** `crates/worktrees_panel/src/worktrees_panel_settings.rs`
- `dock`: Left or Right positioning
- `default_width`: Panel width in pixels
- `button`: Show/hide dock button

### Phase 4: Workspace Switching

Implemented worktree switching via `open_paths()`:
- Clicking a worktree opens that path
- Reuses existing window if one exists for that path
- Creates new window if needed
- Registered keyboard actions (SwitchToWorktree1-5) for future keybinding support

### Phase 6: Git Worktree Integration

- Automatic scanning of git worktrees when workspace opens
- Uses `Repository::worktrees()` API to discover worktrees
- Branch names extracted from git worktree info
- Non-git repositories show "(no git)" indicator and disable worktree features
- Validates worktree paths and removes missing entries

## Files Changed

### New Files
- `crates/workspace/src/worktree_registry.rs`
- `crates/worktrees_panel/Cargo.toml`
- `crates/worktrees_panel/src/worktrees_panel.rs`
- `crates/worktrees_panel/src/worktrees_panel_settings.rs`

### Modified Files
- `crates/workspace/src/workspace.rs` - Added worktree_registry field and initialization
- `crates/workspace/Cargo.toml` - Added chrono dependency
- `crates/zed/src/zed.rs` - Registered panel and init
- `crates/zed/Cargo.toml` - Added worktrees_panel dependency
- `crates/settings_content/src/workspace.rs` - Added WorktreesPanelSettingsContent
- `crates/settings_content/src/settings_content.rs` - Added worktrees_panel field
- `crates/settings/src/vscode_import.rs` - Added worktrees_panel field
- `Cargo.toml` (root) - Added worktrees_panel to workspace

## Not Implemented (Future Work)

### Phase 3: Slot State Persistence
- SerializedWorkspaceSlot for saving/restoring pane layout per worktree
- Database storage for worktree registry state

### Phase 5: Agent Chat Association
- Associating agent threads with specific worktree slots
- Filtering agent history by current worktree

### Phase 8: Polish
- Default keybindings (cmd-1 through cmd-5 conflict with existing bindings)
- Status bar integration showing current worktree branch
- "Add worktree" action from panel

## Architecture Notes

The implementation follows a window-scoped model:
- Each Workspace owns one WorktreeRegistry
- Registry tracks worktrees for the single repo open in that window
- No global registry - each window is independent
- Worktree switching currently opens paths (may reuse or create windows)

True in-window Project swapping was deferred due to complexity:
- Would require rebinding all editors to new Project
- LSP restart/migration
- Subscription management
- The current approach provides value while being significantly simpler

## Usage

The panel appears in the dock and shows all git worktrees for the current repository. Click any worktree to open it. The panel automatically discovers worktrees when opening a git repository.
