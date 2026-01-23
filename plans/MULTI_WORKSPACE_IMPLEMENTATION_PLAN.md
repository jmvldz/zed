# Multi-Worktree Panel Implementation Plan

## Overview

This plan adds a per-window panel for multiple git worktrees that belong to the
current repo. Each window continues to represent a single repo (current
behavior); opening a new window opens a new repo. The panel enables switching
between worktrees for the same repo within a window.

## Single-Repo Per Window Scope

- No multi-repo registry inside a window.
- No single-window enforcement; opening new windows remains unchanged.
- Worktree panel only shows worktrees for the current repo.
- Worktree registry persistence is shared per repo identity; active slot selection
  remains per window.

## Architecture Diagram

```
┌─────────────────────────────────────────────────────────────────────┐
│                              Window                                 │
├─────────────────┬───────────────────────────────────────────────────┤
│  WorktreesPanel │                    Workspace                        │
│  ┌────────────┐ │  ┌─────────────────────────────────────────────┐  │
│  │ main       │ │  │  TabBar: [Agent] [file.rs] [Repo inquiry]  │  │
│  │ feature-x  │ │  ├─────────────────────────────────────────────┤  │
│  │ bugfix-1   │◀│  │                                             │  │
│  └────────────┘ │  │              Editor / Agent View            │  │
│                 │  │                                             │  │
│                 │  └─────────────────────────────────────────────┘  │
│                 │                                                    │
│                 │  ┌─────────────────────────────────────────────┐  │
└─────────────────┴──┤        Agent Panel (filtered by slot)       │──┘
                     └─────────────────────────────────────────────┘
```

## Data Model

```
WorktreeRegistry (Window-scoped Entity stored on Workspace)
├── repo_identity_path: PathBuf
├── repo_root_path: PathBuf
├── worktrees: Vec<WorktreeEntry>
├── slots: HashMap<WorktreeSlotId, WorktreeSlot>
└── db: WorktreeRegistryDb

WorktreeEntry
├── slot_id: WorktreeSlotId
├── worktree_path: PathBuf
├── branch_name: SharedString
├── last_accessed: DateTime<Utc>
└── agent_chat_count: usize

WorktreeSlot
├── slot_id: WorktreeSlotId
├── worktree_path: PathBuf
├── state: SlotState (Active | Cached | Unloaded)
├── project: Option<Entity<Project>>
└── serialized: Option<SerializedWorkspaceSlot>

Note: repo_identity_path uses the git common dir when available, and falls back
to repo_root_path for non-git folders. WorktreeSlotId is derived from the git
worktree gitdir path (GitRepository::path) when available, otherwise from the
canonical worktree path. The active slot ID lives on the Workspace
(window-scoped), not global.
```

---

## Phase 1: WorktreeRegistry Foundation

**Goal:** Create the window-scoped data layer for tracking worktrees in the
current repo.

### 1.1 Create WorktreeRegistry Module

**File:** `crates/workspace/src/worktree_registry.rs` (new)

- Store repo_root_path and worktree entries for the current repo.
- Store repo_identity_path for stable grouping across worktree roots.
- Provide methods:
  - `add_worktree` and `remove_worktree` for registry updates
  - `scan_repo_worktrees` for discovery via GitRepository
  - `set_active_slot` to update last_accessed and emit events
- Derive WorktreeSlotId from git worktree gitdir path (or canonical worktree
  path for non-git), so ids are stable across restarts.

### 1.2 Create Registry Database

**File:** `crates/workspace/src/worktree_registry_db.rs` (new)

- Persist worktree entries by repo_identity_path and WorktreeSlotId.
- Load entries on Workspace initialization for the repo root.
- Errors propagate with `Result` (no panicking).
- If multiple windows open the same repo, last_accessed and slot state use a
  last-writer-wins model for shared fields (worktree list, chat counts).

### 1.3 Integrate into Workspace

**File:** `crates/workspace/src/workspace.rs` (modify)

- Add `worktree_registry: Entity<WorktreeRegistry>` to Workspace.
- Initialize registry when a Workspace is created for a repo root path and
  derive repo_identity_path from the git common dir when available.
- Replace any global registry references with window-scoped usage.

**Acceptance Criteria for Phase 1:**
- WorktreeRegistry tracks worktrees for the current repo.
- Active slot changes update last_accessed and emit events.
- Registry data is reloaded for the repo root on restart.

---

## Phase 2: WorktreesPanel UI

**Goal:** Add a panel listing worktrees for the current repo.

### 2.1 Create WorktreesPanel Crate

**File:** `crates/worktrees_panel/Cargo.toml` (new)

```toml
[package]
name = "worktrees_panel"
version = "0.1.0"
edition = "2024"

[lib]
path = "src/worktrees_panel.rs"

[dependencies]
gpui.workspace = true
workspace.workspace = true
ui.workspace = true
settings.workspace = true
git.workspace = true
util.workspace = true
serde.workspace = true
```

### 2.2 Panel Implementation

**File:** `crates/worktrees_panel/src/worktrees_panel.rs` (new)

- Show the current repo name/path in the header.
- List worktrees for the current repo (branch name + last accessed).
- Order list with the current worktree first, then sort by branch_name and
  worktree_path for stability.
- Highlight the active slot.
- Click to switch to a worktree slot.
- Provide "Add worktree" action (if repo is git).
- Keyboard shortcuts (cmd-1 to cmd-5) switch within the list order.

### 2.3 Register Panel

**File:** `crates/zed/src/zed.rs` (modify)

- Register WorktreesPanel when the feature flag or setting is enabled.

**Acceptance Criteria for Phase 2:**
- Panel renders a list of worktrees for the current repo.
- Active worktree is visually highlighted.
- Clicking a worktree switches slots.
- Add worktree action is visible when git is available.

---

## Phase 3: Worktree Slot State Persistence

**Goal:** Serialize and restore workspace layout per worktree slot.

### 3.1 Define Serialized Slot State

**File:** `crates/workspace/src/persistence/model.rs` (modify)

- Add `SerializedWorkspaceSlot` (pane layout, docks, window state).

### 3.2 Add Slot Persistence Queries

**File:** `crates/workspace/src/worktree_registry_db.rs` (modify)

- Save and load serialized slot state per WorktreeSlotId and window_id (or
  workspace_id) to avoid cross-window clobbering.

### 3.3 Integrate with WorktreeSlot

**File:** `crates/workspace/src/worktree_registry.rs` (modify)

- Add `save_state` and `is_loaded` helpers on WorktreeSlot.

**Acceptance Criteria for Phase 3:**
- Slot state is saved and restored for a worktree slot.
- Dock visibility and pane layout restore when switching slots.

---

## Phase 4: Workspace Switching Core

**Goal:** Switch between worktree slots within the same repo.

### 4.1 Add Switching Method to Workspace

**File:** `crates/workspace/src/workspace.rs` (modify)

- `switch_to_slot` saves current slot state, swaps Project, and restores layout.
- Shutdown and restart LSPs during the swap as needed.
- Update WorktreeRegistry slot states (Active/Cached/Unloaded).
- Stop or park project-scoped background tasks for the old slot when it becomes
  inactive, and ensure they resume only when that slot is re-activated.

### 4.2 Handle Unsaved Changes

**File:** `crates/workspace/src/workspace.rs` (modify)

- Prompt user if there are unsaved changes before switching slots.

### 4.3 Cached Project Cleanup

**File:** `crates/workspace/src/worktree_registry.rs` (modify)

- Drop cached projects after a timeout to avoid memory growth.

**Acceptance Criteria for Phase 4:**
- Switching changes the active slot without changing the repo.
- Current slot state is saved before switching.
- Target slot state is restored after switching.
- LSPs, subscriptions, and project-scoped background tasks are re-bound to the
  new Project without leaking updates from inactive slots.

---

## Phase 5: Agent Chat Worktree Association

**Goal:** Associate agent chats with specific worktree slots.

### 5.1 Add Worktree Slot ID to Thread Storage

**File:** `crates/agent/src/db.rs` (modify)

- Add `worktree_slot_id` to thread metadata and queries (stable id derived from
  gitdir/canonical path).
- If an existing `workspace_slot_id` column exists, keep it and treat it as the
  worktree slot ID to avoid a breaking migration.

### 5.2 Filter Agent History by Worktree

**File:** `crates/agent_ui/src/acp/thread_history.rs` (modify)

- Filter session list using the current worktree slot id.

### 5.3 Update Agent Chat Count in Registry

**File:** `crates/workspace/src/worktree_registry.rs` (modify)

- Update per-worktree chat count for badge display.

**Acceptance Criteria for Phase 5:**
- Agent sessions filter by the current worktree slot.
- Worktree panel shows chat count badges per worktree.

---

## Phase 6: Git Worktree Integration

**Goal:** Create and discover git worktrees for the current repo.

### 6.1 Create Worktree from Panel

**File:** `crates/worktrees_panel/src/worktrees_panel.rs` (modify)

- Prompt for branch name and destination directory.
- Use GitRepository API to create a worktree.
- Add the worktree to the registry.

### 6.2 Scan Existing Worktrees on Repo Open

**File:** `crates/workspace/src/worktree_registry.rs` (modify)

- Discover existing worktrees for the repo root on Workspace initialization.

### 6.3 Watch Worktree Changes

**File:** `crates/workspace/src/worktree_registry.rs` (modify)

- Watch the git worktree directory (e.g. `<common_dir>/worktrees`) and rescan
  worktrees on changes with a short debounce.

### 6.4 Non-Git Repo Behavior

- If the repo root is not a git repository, show a single slot labeled
  "Workspace" with a branch label like "(no git)" and disable worktree
  creation with a clear message.

**Acceptance Criteria for Phase 6:**
- Worktree creation uses the git worktree API and updates the registry.
- Existing worktrees are discovered for the current repo.
- Worktree list updates when worktrees are added or removed outside Zed.
- Non-git repos do not break the panel.

---

## Phase 7: Settings and Feature Flag

**Goal:** Gate the worktree panel behind a setting and feature flag.

### 7.1 Add Settings Schema

**File:** `crates/settings_content/src/workspace.rs` (modify)

- Add `worktree_panel: Option<bool>` to workspace settings.
- If `multi_workspace_mode` already exists, accept it as an alias for
  `worktree_panel` to preserve compatibility.

### 7.2 Add Feature Flag

**File:** `crates/feature_flags/src/flags.rs` (modify)

- Add `WorktreePanelFeatureFlag`.

### 7.3 Conditional Panel Registration

**File:** `crates/zed/src/zed.rs` (modify)

- Register WorktreesPanel only when enabled.

### 7.4 Add Default Setting

**File:** `assets/settings/default.json` (modify)

- Default `worktree_panel` to `false`.

**Acceptance Criteria for Phase 7:**
- Panel is hidden by default.
- Panel registers when setting or feature flag is enabled.

---

## Phase 8: Polish and Edge Cases

**Goal:** Handle edge cases and improve UX for a worktree-focused panel.

### 8.1 Handle Deleted Worktrees

**File:** `crates/workspace/src/worktree_registry.rs` (modify)

- Validate worktree paths on startup and remove missing entries.

### 8.2 Status Bar Integration

**File:** `crates/workspace/src/status_bar.rs` (modify)

- Show current worktree branch in the status bar.

### 8.3 Keyboard Navigation

**File:** `crates/worktrees_panel/src/worktrees_panel.rs` (modify)

- cmd-1 through cmd-5 switch among worktrees in list order.

### 8.4 Time-Ago Formatting

**File:** `crates/worktrees_panel/src/worktrees_panel.rs` (modify)

- Display last-accessed times in a concise format.

**Acceptance Criteria for Phase 8:**
- Deleted worktrees are removed from the registry.
- Status bar shows the active worktree branch name.
- Keyboard shortcuts switch worktrees within the repo.
- Last-accessed times are readable and consistent.

---

## Migration Path

- Feature flag off by default, no change to current behavior.
- Opt-in via settings enables the worktree panel for a window.
- Existing window behavior (one repo per window) remains unchanged.

---

## File Summary

| Phase | New Files | Modified Files |
|-------|-----------|----------------|
| 1 | `worktree_registry.rs`, `worktree_registry_db.rs` | `workspace.rs` |
| 2 | `crates/worktrees_panel/*` | `zed.rs` |
| 3 | - | `persistence/model.rs`, `worktree_registry_db.rs` |
| 4 | - | `workspace.rs`, `worktree_registry.rs` |
| 5 | - | `agent/db.rs`, `thread_history.rs`, `worktree_registry.rs` |
| 6 | - | `worktrees_panel.rs`, `worktree_registry.rs` |
| 7 | - | `settings_content/workspace.rs`, `flags.rs`, `default.json`, `zed.rs` |
| 8 | - | `worktree_registry.rs`, `status_bar.rs`, `worktrees_panel.rs` |

---

## Estimated Complexity

| Phase | Complexity | Dependencies |
|-------|------------|--------------|
| 1 | Medium | None |
| 2 | Medium | Phase 1 |
| 3 | Low | Phase 1 |
| 4 | High | Phase 1, 3 |
| 5 | Medium | Phase 1, 4 |
| 6 | Low | Phase 1, 2 |
| 7 | Low | Phase 2 |
| 8 | Medium | Phases 1-6 |

**Critical Path:** Phase 1 -> Phase 3 -> Phase 4 (switching is the hardest part)

**Parallelizable:** Phase 2, Phase 5, Phase 6, Phase 7 can proceed after Phase 1
