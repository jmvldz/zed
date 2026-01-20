# Agent-Scoped Worktree Plan

## Goals
- Each agent panel switches the entire window UI to its own repo worktree.
- Changes can be applied back to the primary repo via an explicit action.

## Assumptions
- Use git worktrees (not full clones) for speed and shared objects.
- One worktree per agent session (not per agent type).
- Single active workspace per window; switching agent panel swaps workspace.

## Phase 1: Discovery and Hook Points (exact files)
- Agent selection and panel switching:
  - `crates/agent_ui/src/agent_panel.rs`:
    - `fn history_kind_for_selected_agent`
    - `fn open_history`
    - `fn new_agent_thread`
    - agent selection callbacks
  - `crates/agent_ui/src/agent_chat_content.rs`:
    - `fn external_thread`
    - `fn set_active_view`
    - `fn new_agent_thread`
- Thread creation and connection:
  - `crates/agent_ui/src/acp/thread_view.rs`:
    - connection setup and `session_list` wiring
    - agent session lifecycle hooks
- Workspace/project plumbing:
  - `crates/workspace/` (identify entry for opening a new workspace/project)
  - `crates/project/src/project.rs` (project construction)
  - `crates/client/src/workspaces.rs` or equivalent workspace store (if present)
- Settings toggles:
  - `crates/settings_content/src/agent.rs` (new setting schema)
  - `crates/settings/src/settings.rs` (default values + parsing)
- Persisted metadata location (session id → worktree path → branch):
  - `crates/project/src/agent_server_store.rs` or new module under `crates/project/src/`
  - `paths::data_dir()` in `crates/paths/src/paths.rs`

## Phase 2: Worktree Lifecycle (exact changes)
- New module for worktree management (preferred):
  - `crates/project/src/agent_worktrees.rs`
    - `AgentWorktreeManager`:
      - `create(session_id, repo_root, cx) -> Result<PathBuf>`
      - `get(session_id) -> Option<PathBuf>`
      - `remove(session_id) -> Result<()>`
    - Persist mapping in a small JSON or SQLite table under:
      - `paths::data_dir()/agent_worktrees/registry.json`
- Git worktree creation:
  - Use existing git helpers if available, else shell out via existing git wrapper.
  - Worktree path:
    - `paths::data_dir()/agent_worktrees/<repo_hash>/<session_id>`
  - Branch name:
    - `agent/<session_id>` (created at worktree creation)
- Integration points:
  - `crates/project/src/project.rs`: expose repo root and git metadata needed for worktree creation
  - `crates/git/` or equivalent git wrapper for:
    - `git worktree add -b agent/<session_id> <path> <base_ref>`
    - `git worktree list` for cleanup/validation

## Phase 3: UI Switching (exact changes)
- On agent panel selection:
  - `crates/agent_ui/src/agent_panel.rs`:
    - Hook agent selection handler to call a new `switch_to_agent_workspace(...)`.
  - `crates/agent_ui/src/agent_chat_content.rs`:
    - In `external_thread(...)`, after selecting agent and before connection:
      - Resolve or create worktree for session id (if enabled).
      - Request workspace switch for the window.
- Workspace switching API:
  - If existing: use `WorkspaceStore` / `Workspace::open` API.
  - If missing: add method in `crates/workspace/` to replace window’s workspace:
    - `Workspace::switch_in_window(new_workspace, window, cx)`
  - Ensure `project` + LSP + git views rebind cleanly.
- UI state:
  - Ensure active view resets to agent thread in the new workspace.
  - Keep a minimal back-stack for switching between main repo and agent worktree.

## Phase 4: Apply-Back Workflow (exact changes)
- UI action:
  - `crates/agent_ui/src/agent_panel.rs` or `crates/agent_ui/src/acp/thread_view.rs`:
    - Add `ApplyToMain` action button for external agents.
    - Only visible when worktree mode is enabled and session has worktree.
- Git operations:
  - New helper in `crates/project/src/agent_worktrees.rs` or `crates/git/`:
    - `merge_worktree_branch(session_id, main_repo_path) -> Result<MergeOutcome>`
    - `create_patch(session_id) -> Result<String>` (fallback)
    - `apply_patch(main_repo_path, patch) -> Result<()>`
- Error handling:
  - If main repo dirty: prompt to stash or abort.
  - If merge conflict: surface a conflict UI and allow manual resolution.

## Phase 5: Settings/Flags (exact changes)
- Settings schema:
  - `crates/settings_content/src/agent.rs`:
    - Add `agent.worktree_sessions: bool`
  - `crates/settings/src/settings.rs`:
    - Default false
- Optional feature flag:
  - `crates/feature_flags/` (if required for gated rollout)

## Phase 6: Telemetry & Tests (exact changes)
- Logging:
  - `crates/agent_servers/src/acp.rs` or `crates/agent_ui/src/agent_panel.rs`:
    - log session id + worktree path on creation/switch
  - `crates/project/src/agent_worktrees.rs`:
    - log apply/merge outcomes
- Tests:
  - `crates/agent_ui/src/acp/thread_view.rs`:
    - add a test that ensures history visibility with worktrees enabled
  - `crates/project/src/agent_worktrees.rs`:
    - unit tests for worktree creation and registry
  - Use GPUI timers per repo guidelines when needed

## Open Questions
- Should worktrees be per session or per agent type?
- Should worktrees be cleaned automatically on session delete?
- How to handle non-git folders or detached HEAD states?
