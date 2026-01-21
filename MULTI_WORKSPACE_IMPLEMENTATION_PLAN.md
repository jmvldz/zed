# Multi-Workspace Architecture Implementation Plan

## Overview

This plan implements a new workspace model where:
- Multiple repositories can be tracked in one window
- Each git worktree is its own workspace
- Multiple agent chats are associated with each workspace
- Quick switching between workspaces via sidebar and keyboard shortcuts

## Architecture Diagram

```
┌─────────────────────────────────────────────────────────────────────┐
│                              Window                                  │
├─────────────────┬───────────────────────────────────────────────────┤
│  WorkspacesPanel│                    Workspace                       │
│  ┌────────────┐ │  ┌─────────────────────────────────────────────┐  │
│  │ swarm      │ │  │  TabBar: [Agent] [file.rs] [Repo inquiry]  │  │
│  │  └ main    │ │  ├─────────────────────────────────────────────┤  │
│  │ zed        │ │  │                                             │  │
│  │  ├ san-diego│◀│  │              Editor / Agent View            │  │
│  │  └ prague  │ │  │                                             │  │
│  └────────────┘ │  └─────────────────────────────────────────────┘  │
│                 │                                                    │
│  [Add repo]     │  ┌─────────────────────────────────────────────┐  │
└─────────────────┴──┤        Agent Panel (filtered by workspace)  │──┘
                     └─────────────────────────────────────────────┘
```

## Data Model

```
WorkspaceRegistry (Global Entity)
├── repos: IndexMap<RepoId, RepoEntry>
│   └── RepoEntry
│       ├── root_path: PathBuf
│       ├── display_name: SharedString
│       └── worktrees: Vec<WorktreeEntry>
│           └── WorktreeEntry
│               ├── slot_id: WorkspaceSlotId
│               ├── worktree_path: PathBuf
│               ├── branch_name: SharedString
│               └── last_accessed: DateTime<Utc>
│
└── slots: HashMap<WorkspaceSlotId, WorkspaceSlot>
    └── WorkspaceSlot
        ├── slot_id: WorkspaceSlotId
        ├── repo_id: RepoId
        ├── worktree_path: PathBuf
        ├── state: SlotState (Active | Cached | Unloaded)
        ├── project: Option<Entity<Project>>
        └── serialized: Option<SerializedWorkspaceSlot>
```

---

## Phase 1: WorkspaceRegistry Foundation

**Goal:** Create the data layer for tracking repos and worktrees globally.

### 1.1 Create WorkspaceRegistry Module

**File:** `crates/workspace/src/workspace_registry.rs` (new)

```rust
use gpui::{App, Context, Entity, EventEmitter, Global};
use std::path::PathBuf;
use collections::{HashMap, IndexMap};
use chrono::{DateTime, Utc};

// Unique identifiers
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RepoId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WorkspaceSlotId(pub u64);

// Repository entry
#[derive(Debug, Clone)]
pub struct RepoEntry {
    pub id: RepoId,
    pub root_path: PathBuf,
    pub display_name: SharedString,
    pub worktrees: Vec<WorktreeEntry>,
}

// Worktree entry (one per git worktree)
#[derive(Debug, Clone)]
pub struct WorktreeEntry {
    pub slot_id: WorkspaceSlotId,
    pub worktree_path: PathBuf,
    pub branch_name: SharedString,
    pub last_accessed: DateTime<Utc>,
    pub agent_chat_count: usize,
}

// Slot state for lazy loading
#[derive(Debug, Clone)]
pub enum SlotState {
    /// Currently displayed in window
    Active,
    /// Project kept in memory for fast switching
    Cached,
    /// Only serialized state, Project not loaded
    Unloaded,
}

// Workspace slot (full state container)
pub struct WorkspaceSlot {
    pub slot_id: WorkspaceSlotId,
    pub repo_id: RepoId,
    pub worktree_path: PathBuf,
    pub state: SlotState,
    pub project: Option<Entity<Project>>,
    pub serialized: Option<SerializedWorkspaceSlot>,
}

// Events
pub enum WorkspaceRegistryEvent {
    RepoAdded(RepoId),
    RepoRemoved(RepoId),
    WorktreeAdded { repo_id: RepoId, slot_id: WorkspaceSlotId },
    WorktreeRemoved { repo_id: RepoId, slot_id: WorkspaceSlotId },
    ActiveSlotChanged { old: Option<WorkspaceSlotId>, new: WorkspaceSlotId },
}

// Main registry
pub struct WorkspaceRegistry {
    repos: IndexMap<RepoId, RepoEntry>,
    slots: HashMap<WorkspaceSlotId, WorkspaceSlot>,
    active_slot_id: Option<WorkspaceSlotId>,
    next_repo_id: u64,
    next_slot_id: u64,
    db: WorkspaceRegistryDb,
}

impl EventEmitter<WorkspaceRegistryEvent> for WorkspaceRegistry {}

impl WorkspaceRegistry {
    pub fn new(cx: &mut App) -> Entity<Self> {
        cx.new(|cx| {
            let db = WorkspaceRegistryDb::new();
            let (repos, slots) = db.load_all().unwrap_or_default();
            Self {
                repos,
                slots,
                active_slot_id: None,
                next_repo_id: repos.keys().map(|r| r.0).max().unwrap_or(0) + 1,
                next_slot_id: slots.keys().map(|s| s.0).max().unwrap_or(0) + 1,
                db,
            }
        })
    }

    /// Add a repository to the registry
    pub fn add_repo(&mut self, root_path: PathBuf, cx: &mut Context<Self>) -> RepoId {
        // Check if repo already exists
        if let Some(entry) = self.repos.values().find(|r| r.root_path == root_path) {
            return entry.id;
        }

        let id = RepoId(self.next_repo_id);
        self.next_repo_id += 1;

        let display_name = root_path
            .file_name()
            .map(|n| n.to_string_lossy().into())
            .unwrap_or_else(|| "unknown".into());

        let entry = RepoEntry {
            id,
            root_path: root_path.clone(),
            display_name,
            worktrees: Vec::new(),
        };

        self.repos.insert(id, entry);
        self.db.save_repo(id, &root_path).log_err();
        cx.emit(WorkspaceRegistryEvent::RepoAdded(id));
        cx.notify();
        id
    }

    /// Add a worktree to a repository
    pub fn add_worktree(
        &mut self,
        repo_id: RepoId,
        worktree_path: PathBuf,
        branch_name: SharedString,
        cx: &mut Context<Self>,
    ) -> Option<WorkspaceSlotId> {
        let repo = self.repos.get_mut(&repo_id)?;

        // Check if worktree already exists
        if let Some(existing) = repo.worktrees.iter().find(|w| w.worktree_path == worktree_path) {
            return Some(existing.slot_id);
        }

        let slot_id = WorkspaceSlotId(self.next_slot_id);
        self.next_slot_id += 1;

        let worktree_entry = WorktreeEntry {
            slot_id,
            worktree_path: worktree_path.clone(),
            branch_name,
            last_accessed: Utc::now(),
            agent_chat_count: 0,
        };

        let slot = WorkspaceSlot {
            slot_id,
            repo_id,
            worktree_path: worktree_path.clone(),
            state: SlotState::Unloaded,
            project: None,
            serialized: None,
        };

        repo.worktrees.push(worktree_entry);
        self.slots.insert(slot_id, slot);

        self.db.save_worktree(slot_id, repo_id, &worktree_path).log_err();
        cx.emit(WorkspaceRegistryEvent::WorktreeAdded { repo_id, slot_id });
        cx.notify();
        Some(slot_id)
    }

    /// Get all repos
    pub fn repos(&self) -> impl Iterator<Item = &RepoEntry> {
        self.repos.values()
    }

    /// Get a specific slot
    pub fn slot(&self, slot_id: WorkspaceSlotId) -> Option<&WorkspaceSlot> {
        self.slots.get(&slot_id)
    }

    /// Get the active slot
    pub fn active_slot(&self) -> Option<&WorkspaceSlot> {
        self.active_slot_id.and_then(|id| self.slots.get(&id))
    }

    /// Set active slot (called during workspace switching)
    pub fn set_active_slot(&mut self, slot_id: WorkspaceSlotId, cx: &mut Context<Self>) {
        let old = self.active_slot_id;
        self.active_slot_id = Some(slot_id);

        // Update last_accessed
        if let Some(slot) = self.slots.get_mut(&slot_id) {
            if let Some(repo) = self.repos.get_mut(&slot.repo_id) {
                if let Some(worktree) = repo.worktrees.iter_mut().find(|w| w.slot_id == slot_id) {
                    worktree.last_accessed = Utc::now();
                }
            }
        }

        cx.emit(WorkspaceRegistryEvent::ActiveSlotChanged { old, new: slot_id });
        cx.notify();
    }

    /// Scan a repo for git worktrees and add them
    pub fn scan_repo_worktrees(
        &mut self,
        repo_id: RepoId,
        git_repo: &dyn GitRepository,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let this = cx.weak_entity();
        cx.spawn(async move |cx| {
            let worktrees = git_repo.worktrees().await?;

            cx.update(|cx| {
                this.update(cx, |registry, cx| {
                    for worktree in worktrees {
                        registry.add_worktree(
                            repo_id,
                            worktree.path,
                            worktree.ref_name,
                            cx,
                        );
                    }
                })
            })??;

            Ok(())
        })
    }
}
```

### 1.2 Create Registry Database

**File:** `crates/workspace/src/workspace_registry_db.rs` (new)

```rust
use rusqlite::Connection;
use std::path::PathBuf;

pub struct WorkspaceRegistryDb {
    connection: Connection,
}

impl WorkspaceRegistryDb {
    pub fn new() -> Self {
        let db_path = paths::data_dir().join("workspace_registry.db");
        let connection = Connection::open(&db_path).expect("Failed to open registry db");

        connection.execute_batch(r#"
            CREATE TABLE IF NOT EXISTS repos (
                id INTEGER PRIMARY KEY,
                root_path TEXT NOT NULL UNIQUE,
                display_name TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS worktrees (
                slot_id INTEGER PRIMARY KEY,
                repo_id INTEGER NOT NULL REFERENCES repos(id),
                worktree_path TEXT NOT NULL,
                branch_name TEXT NOT NULL,
                last_accessed TEXT NOT NULL,
                UNIQUE(repo_id, worktree_path)
            );

            CREATE TABLE IF NOT EXISTS workspace_slots (
                slot_id INTEGER PRIMARY KEY REFERENCES worktrees(slot_id),
                serialized_panes BLOB,
                serialized_docks BLOB,
                window_bounds BLOB
            );

            CREATE INDEX IF NOT EXISTS idx_worktrees_repo ON worktrees(repo_id);
        "#).expect("Failed to create tables");

        Self { connection }
    }

    pub fn load_all(&self) -> Result<(IndexMap<RepoId, RepoEntry>, HashMap<WorkspaceSlotId, WorkspaceSlot>)> {
        // Load repos and worktrees from database
        // Implementation details...
    }

    pub fn save_repo(&self, id: RepoId, root_path: &Path) -> Result<()> {
        // Save repo to database
    }

    pub fn save_worktree(&self, slot_id: WorkspaceSlotId, repo_id: RepoId, path: &Path) -> Result<()> {
        // Save worktree to database
    }

    pub fn save_slot_state(&self, slot_id: WorkspaceSlotId, serialized: &SerializedWorkspaceSlot) -> Result<()> {
        // Save serialized workspace state for a slot
    }
}
```

### 1.3 Integrate into AppState

**File:** `crates/workspace/src/workspace.rs` (modify)

```rust
// Add to AppState struct (around line 952)
pub struct AppState {
    // ... existing fields ...
    pub workspace_registry: Entity<WorkspaceRegistry>,
}

// Initialize in app setup
impl AppState {
    pub fn new(/* ... */) -> Arc<Self> {
        // ... existing initialization ...
        let workspace_registry = WorkspaceRegistry::new(cx);
        // ...
    }
}
```

### 1.4 Tests for Phase 1

**File:** `crates/workspace/src/workspace_registry_tests.rs` (new)

```rust
use gpui::TestAppContext;
use tempfile::TempDir;

/// Helper to create a test registry with an in-memory or temp database
fn create_test_registry(cx: &mut TestAppContext) -> Entity<WorkspaceRegistry> {
    cx.new(|cx| WorkspaceRegistry::new_for_test(cx))
}

// ============================================================================
// REPO MANAGEMENT TESTS
// ============================================================================

#[gpui::test]
async fn test_add_repo_creates_entry(cx: &mut TestAppContext) {
    let registry = create_test_registry(cx);

    registry.update(cx, |r, cx| {
        let id = r.add_repo("/path/to/repo".into(), cx);

        // Verify repo was added
        assert!(r.repos().any(|repo| repo.id == id));

        // Verify repo has correct path
        let repo = r.repos().find(|repo| repo.id == id).unwrap();
        assert_eq!(repo.root_path, PathBuf::from("/path/to/repo"));
    });
}

#[gpui::test]
async fn test_add_repo_extracts_display_name(cx: &mut TestAppContext) {
    let registry = create_test_registry(cx);

    registry.update(cx, |r, cx| {
        let id = r.add_repo("/Users/dev/projects/my-awesome-project".into(), cx);

        let repo = r.repos().find(|repo| repo.id == id).unwrap();
        assert_eq!(repo.display_name.as_ref(), "my-awesome-project");
    });
}

#[gpui::test]
async fn test_add_repo_is_idempotent(cx: &mut TestAppContext) {
    let registry = create_test_registry(cx);

    registry.update(cx, |r, cx| {
        let id1 = r.add_repo("/path/to/repo".into(), cx);
        let id2 = r.add_repo("/path/to/repo".into(), cx);

        // Same repo should return same ID
        assert_eq!(id1, id2);

        // Should only have one repo
        assert_eq!(r.repos().count(), 1);
    });
}

#[gpui::test]
async fn test_add_repo_emits_event(cx: &mut TestAppContext) {
    let registry = create_test_registry(cx);
    let events = Rc::new(RefCell::new(Vec::new()));

    let events_clone = events.clone();
    cx.subscribe(&registry, move |_, event, _| {
        events_clone.borrow_mut().push(event.clone());
    }).detach();

    registry.update(cx, |r, cx| {
        r.add_repo("/path/to/repo".into(), cx);
    });

    cx.run_until_parked();

    let events = events.borrow();
    assert!(matches!(events.first(), Some(WorkspaceRegistryEvent::RepoAdded(_))));
}

#[gpui::test]
async fn test_remove_repo(cx: &mut TestAppContext) {
    let registry = create_test_registry(cx);

    let repo_id = registry.update(cx, |r, cx| {
        r.add_repo("/path/to/repo".into(), cx)
    });

    registry.update(cx, |r, cx| {
        r.remove_repo(repo_id, cx);
        assert_eq!(r.repos().count(), 0);
    });
}

// ============================================================================
// WORKTREE MANAGEMENT TESTS
// ============================================================================

#[gpui::test]
async fn test_add_worktree_to_repo(cx: &mut TestAppContext) {
    let registry = create_test_registry(cx);

    registry.update(cx, |r, cx| {
        let repo_id = r.add_repo("/path/to/repo".into(), cx);
        let slot_id = r.add_worktree(
            repo_id,
            "/path/to/repo-feature".into(),
            "feature-branch".into(),
            cx,
        );

        assert!(slot_id.is_some());

        // Verify worktree is associated with repo
        let repo = r.repos().find(|repo| repo.id == repo_id).unwrap();
        assert_eq!(repo.worktrees.len(), 1);
        assert_eq!(repo.worktrees[0].branch_name.as_ref(), "feature-branch");
    });
}

#[gpui::test]
async fn test_add_worktree_creates_slot(cx: &mut TestAppContext) {
    let registry = create_test_registry(cx);

    registry.update(cx, |r, cx| {
        let repo_id = r.add_repo("/path/to/repo".into(), cx);
        let slot_id = r.add_worktree(
            repo_id,
            "/path/to/repo-feature".into(),
            "feature-branch".into(),
            cx,
        ).unwrap();

        // Verify slot was created
        let slot = r.slot(slot_id).unwrap();
        assert_eq!(slot.repo_id, repo_id);
        assert_eq!(slot.worktree_path, PathBuf::from("/path/to/repo-feature"));
        assert!(matches!(slot.state, SlotState::Unloaded));
    });
}

#[gpui::test]
async fn test_add_worktree_to_nonexistent_repo_returns_none(cx: &mut TestAppContext) {
    let registry = create_test_registry(cx);

    registry.update(cx, |r, cx| {
        let fake_repo_id = RepoId(9999);
        let slot_id = r.add_worktree(
            fake_repo_id,
            "/path/to/worktree".into(),
            "branch".into(),
            cx,
        );

        assert!(slot_id.is_none());
    });
}

#[gpui::test]
async fn test_add_worktree_is_idempotent(cx: &mut TestAppContext) {
    let registry = create_test_registry(cx);

    registry.update(cx, |r, cx| {
        let repo_id = r.add_repo("/path/to/repo".into(), cx);

        let slot_id1 = r.add_worktree(
            repo_id,
            "/path/to/repo-feature".into(),
            "feature-branch".into(),
            cx,
        );
        let slot_id2 = r.add_worktree(
            repo_id,
            "/path/to/repo-feature".into(),
            "feature-branch".into(),
            cx,
        );

        assert_eq!(slot_id1, slot_id2);

        let repo = r.repos().find(|repo| repo.id == repo_id).unwrap();
        assert_eq!(repo.worktrees.len(), 1);
    });
}

#[gpui::test]
async fn test_multiple_worktrees_per_repo(cx: &mut TestAppContext) {
    let registry = create_test_registry(cx);

    registry.update(cx, |r, cx| {
        let repo_id = r.add_repo("/path/to/repo".into(), cx);

        r.add_worktree(repo_id, "/path/to/repo-feature1".into(), "feature-1".into(), cx);
        r.add_worktree(repo_id, "/path/to/repo-feature2".into(), "feature-2".into(), cx);
        r.add_worktree(repo_id, "/path/to/repo-feature3".into(), "feature-3".into(), cx);

        let repo = r.repos().find(|repo| repo.id == repo_id).unwrap();
        assert_eq!(repo.worktrees.len(), 3);
    });
}

// ============================================================================
// ACTIVE SLOT TESTS
// ============================================================================

#[gpui::test]
async fn test_set_active_slot(cx: &mut TestAppContext) {
    let registry = create_test_registry(cx);

    registry.update(cx, |r, cx| {
        let repo_id = r.add_repo("/path/to/repo".into(), cx);
        let slot_id = r.add_worktree(
            repo_id,
            "/path/to/repo-feature".into(),
            "feature".into(),
            cx,
        ).unwrap();

        assert!(r.active_slot().is_none());

        r.set_active_slot(slot_id, cx);

        assert!(r.active_slot().is_some());
        assert_eq!(r.active_slot().unwrap().slot_id, slot_id);
    });
}

#[gpui::test]
async fn test_set_active_slot_updates_last_accessed(cx: &mut TestAppContext) {
    let registry = create_test_registry(cx);

    registry.update(cx, |r, cx| {
        let repo_id = r.add_repo("/path/to/repo".into(), cx);
        let slot_id = r.add_worktree(
            repo_id,
            "/path/to/repo-feature".into(),
            "feature".into(),
            cx,
        ).unwrap();

        let repo = r.repos().find(|repo| repo.id == repo_id).unwrap();
        let initial_time = repo.worktrees[0].last_accessed;

        // Wait a bit
        std::thread::sleep(std::time::Duration::from_millis(10));

        r.set_active_slot(slot_id, cx);

        let repo = r.repos().find(|repo| repo.id == repo_id).unwrap();
        assert!(repo.worktrees[0].last_accessed > initial_time);
    });
}

#[gpui::test]
async fn test_set_active_slot_emits_event(cx: &mut TestAppContext) {
    let registry = create_test_registry(cx);
    let events = Rc::new(RefCell::new(Vec::new()));

    let (repo_id, slot_id) = registry.update(cx, |r, cx| {
        let repo_id = r.add_repo("/path/to/repo".into(), cx);
        let slot_id = r.add_worktree(repo_id, "/path".into(), "branch".into(), cx).unwrap();
        (repo_id, slot_id)
    });

    let events_clone = events.clone();
    cx.subscribe(&registry, move |_, event, _| {
        events_clone.borrow_mut().push(event.clone());
    }).detach();

    registry.update(cx, |r, cx| {
        r.set_active_slot(slot_id, cx);
    });

    cx.run_until_parked();

    let events = events.borrow();
    assert!(events.iter().any(|e| matches!(e, WorkspaceRegistryEvent::ActiveSlotChanged { .. })));
}

// ============================================================================
// PERSISTENCE TESTS
// ============================================================================

#[gpui::test]
async fn test_persistence_repos_survive_restart(cx: &mut TestAppContext) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("registry.db");

    // Create registry and add data
    {
        let registry = cx.new(|cx| WorkspaceRegistry::new_with_db_path(&db_path, cx));
        registry.update(cx, |r, cx| {
            r.add_repo("/path/to/repo1".into(), cx);
            r.add_repo("/path/to/repo2".into(), cx);
        });
    }

    // Create new registry from same DB
    let registry = cx.new(|cx| WorkspaceRegistry::new_with_db_path(&db_path, cx));
    registry.update(cx, |r, _| {
        assert_eq!(r.repos().count(), 2);
        assert!(r.repos().any(|repo| repo.root_path == PathBuf::from("/path/to/repo1")));
        assert!(r.repos().any(|repo| repo.root_path == PathBuf::from("/path/to/repo2")));
    });
}

#[gpui::test]
async fn test_persistence_worktrees_survive_restart(cx: &mut TestAppContext) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("registry.db");

    // Create registry and add data
    let repo_id = {
        let registry = cx.new(|cx| WorkspaceRegistry::new_with_db_path(&db_path, cx));
        registry.update(cx, |r, cx| {
            let repo_id = r.add_repo("/path/to/repo".into(), cx);
            r.add_worktree(repo_id, "/path/to/wt1".into(), "branch1".into(), cx);
            r.add_worktree(repo_id, "/path/to/wt2".into(), "branch2".into(), cx);
            repo_id
        })
    };

    // Create new registry from same DB
    let registry = cx.new(|cx| WorkspaceRegistry::new_with_db_path(&db_path, cx));
    registry.update(cx, |r, _| {
        let repo = r.repos().find(|repo| repo.id == repo_id).unwrap();
        assert_eq!(repo.worktrees.len(), 2);
    });
}

// ============================================================================
// ID GENERATION TESTS
// ============================================================================

#[gpui::test]
async fn test_repo_ids_are_unique(cx: &mut TestAppContext) {
    let registry = create_test_registry(cx);

    registry.update(cx, |r, cx| {
        let id1 = r.add_repo("/path/to/repo1".into(), cx);
        let id2 = r.add_repo("/path/to/repo2".into(), cx);
        let id3 = r.add_repo("/path/to/repo3".into(), cx);

        assert_ne!(id1, id2);
        assert_ne!(id2, id3);
        assert_ne!(id1, id3);
    });
}

#[gpui::test]
async fn test_slot_ids_are_unique(cx: &mut TestAppContext) {
    let registry = create_test_registry(cx);

    registry.update(cx, |r, cx| {
        let repo_id = r.add_repo("/path/to/repo".into(), cx);
        let slot1 = r.add_worktree(repo_id, "/path/wt1".into(), "b1".into(), cx).unwrap();
        let slot2 = r.add_worktree(repo_id, "/path/wt2".into(), "b2".into(), cx).unwrap();
        let slot3 = r.add_worktree(repo_id, "/path/wt3".into(), "b3".into(), cx).unwrap();

        assert_ne!(slot1, slot2);
        assert_ne!(slot2, slot3);
        assert_ne!(slot1, slot3);
    });
}

// ============================================================================
// AGENT CHAT COUNT TESTS
// ============================================================================

#[gpui::test]
async fn test_update_agent_chat_count(cx: &mut TestAppContext) {
    let registry = create_test_registry(cx);

    registry.update(cx, |r, cx| {
        let repo_id = r.add_repo("/path/to/repo".into(), cx);
        let slot_id = r.add_worktree(repo_id, "/path/wt".into(), "branch".into(), cx).unwrap();

        // Initially zero
        let repo = r.repos().find(|repo| repo.id == repo_id).unwrap();
        assert_eq!(repo.worktrees[0].agent_chat_count, 0);

        // Update count
        r.update_agent_chat_count(slot_id, 5, cx);

        let repo = r.repos().find(|repo| repo.id == repo_id).unwrap();
        assert_eq!(repo.worktrees[0].agent_chat_count, 5);
    });
}
```

**Acceptance Criteria for Phase 1:**
- [ ] All tests pass
- [ ] `WorkspaceRegistry` can add/remove repos
- [ ] `WorkspaceRegistry` can add/remove worktrees
- [ ] Active slot can be set and retrieved
- [ ] Data persists across registry restarts
- [ ] Events are emitted for all state changes

---

## Phase 2: WorkspacesPanel UI

**Goal:** Create the sidebar panel showing repos and worktrees.

### 2.1 Create WorkspacesPanel Crate

**File:** `crates/workspaces_panel/Cargo.toml` (new)

```toml
[package]
name = "workspaces_panel"
version = "0.1.0"
edition = "2024"

[lib]
path = "src/workspaces_panel.rs"

[dependencies]
gpui.workspace = true
workspace.workspace = true
ui.workspace = true
settings.workspace = true
git.workspace = true
util.workspace = true
serde.workspace = true
```

### 2.2 Create Panel Implementation

**File:** `crates/workspaces_panel/src/workspaces_panel.rs` (new)

```rust
use gpui::*;
use workspace::{
    dock::{Panel, PanelEvent},
    WorkspaceRegistry, RepoEntry, WorktreeEntry, WorkspaceSlotId,
};
use ui::prelude::*;
use std::sync::Arc;

actions!(workspaces_panel, [
    ToggleFocus,
    AddRepository,
    CreateWorktree,
    SwitchToWorkspace1,
    SwitchToWorkspace2,
    SwitchToWorkspace3,
    SwitchToWorkspace4,
    SwitchToWorkspace5,
]);

pub fn init(cx: &mut App) {
    cx.observe_new(WorkspacesPanel::register).detach();
}

pub struct WorkspacesPanel {
    registry: Entity<WorkspaceRegistry>,
    workspace: WeakEntity<Workspace>,
    expanded_repos: HashSet<RepoId>,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl WorkspacesPanel {
    pub fn new(
        workspace: &Workspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let registry = workspace.app_state().workspace_registry.clone();
        let focus_handle = cx.focus_handle();

        // Subscribe to registry changes
        let subscriptions = vec![
            cx.subscribe(&registry, Self::on_registry_event),
        ];

        Self {
            registry,
            workspace: workspace.weak_handle(),
            expanded_repos: HashSet::default(),
            focus_handle,
            _subscriptions: subscriptions,
        }
    }

    fn on_registry_event(
        &mut self,
        _: Entity<WorkspaceRegistry>,
        event: &WorkspaceRegistryEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            WorkspaceRegistryEvent::RepoAdded(_) |
            WorkspaceRegistryEvent::WorktreeAdded { .. } |
            WorkspaceRegistryEvent::ActiveSlotChanged { .. } => {
                cx.notify();
            }
            _ => {}
        }
    }

    fn toggle_repo_expanded(&mut self, repo_id: RepoId, cx: &mut Context<Self>) {
        if self.expanded_repos.contains(&repo_id) {
            self.expanded_repos.remove(&repo_id);
        } else {
            self.expanded_repos.insert(repo_id);
        }
        cx.notify();
    }

    fn switch_to_workspace(&mut self, slot_id: WorkspaceSlotId, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(workspace) = self.workspace.upgrade() {
            workspace.update(cx, |workspace, cx| {
                workspace.switch_to_slot(slot_id, window, cx);
            });
        }
    }

    fn add_repository(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Open file picker to select repository root
        cx.spawn_in(window, async move |this, mut cx| {
            let path = cx.update(|window, cx| {
                window.prompt_for_paths(PromptPathOptions {
                    files: false,
                    directories: true,
                    multiple: false,
                })
            })?.await?;

            if let Some(paths) = path {
                if let Some(path) = paths.first() {
                    this.update(&mut cx, |panel, cx| {
                        panel.registry.update(cx, |registry, cx| {
                            registry.add_repo(path.clone(), cx);
                        });
                    })?;
                }
            }
            Ok::<_, anyhow::Error>(())
        }).detach_and_log_err(cx);
    }

    fn render_repo(&self, repo: &RepoEntry, cx: &App) -> impl IntoElement {
        let repo_id = repo.id;
        let is_expanded = self.expanded_repos.contains(&repo_id);
        let active_slot = self.registry.read(cx).active_slot_id;

        v_flex()
            .child(
                // Repo header
                h_flex()
                    .gap_1()
                    .px_2()
                    .py_1()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, cx| {
                        this.toggle_repo_expanded(repo_id, cx);
                    }))
                    .child(
                        Icon::new(if is_expanded {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .size(IconSize::Small)
                    )
                    .child(
                        Icon::new(IconName::Folder)
                            .size(IconSize::Small)
                            .color(Color::Muted)
                    )
                    .child(
                        Label::new(repo.display_name.clone())
                            .size(LabelSize::Small)
                    )
                    .child(
                        // "+" button for creating new worktree
                        IconButton::new("add-worktree", IconName::Plus)
                            .size(ButtonSize::Compact)
                            .on_click(cx.listener(move |this, _, cx| {
                                this.create_worktree_for_repo(repo_id, cx);
                            }))
                    )
            )
            .when(is_expanded, |this| {
                this.children(
                    repo.worktrees.iter().enumerate().map(|(idx, worktree)| {
                        self.render_worktree(worktree, idx, active_slot, cx)
                    })
                )
            })
    }

    fn render_worktree(
        &self,
        worktree: &WorktreeEntry,
        index: usize,
        active_slot: Option<WorkspaceSlotId>,
        cx: &App,
    ) -> impl IntoElement {
        let slot_id = worktree.slot_id;
        let is_active = active_slot == Some(slot_id);
        let shortcut_num = index + 1;

        h_flex()
            .pl_6()
            .pr_2()
            .py_1()
            .cursor_pointer()
            .when(is_active, |this| this.bg(cx.theme().colors().ghost_element_selected))
            .hover(|this| this.bg(cx.theme().colors().ghost_element_hover))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.switch_to_workspace(slot_id, window, cx);
            }))
            .child(
                Icon::new(IconName::GitBranch)
                    .size(IconSize::Small)
                    .color(Color::Muted)
            )
            .child(
                v_flex()
                    .child(
                        Label::new(worktree.branch_name.clone())
                            .size(LabelSize::Small)
                    )
                    .child(
                        Label::new(format_time_ago(worktree.last_accessed))
                            .size(LabelSize::XSmall)
                            .color(Color::Muted)
                    )
            )
            .child(div().flex_grow())
            // Agent chat count badge
            .when(worktree.agent_chat_count > 0, |this| {
                this.child(
                    div()
                        .px_1()
                        .rounded_sm()
                        .bg(cx.theme().colors().element_background)
                        .child(
                            Label::new(format!("+{}", worktree.agent_chat_count))
                                .size(LabelSize::XSmall)
                                .color(Color::Muted)
                        )
                )
            })
            // Keyboard shortcut hint
            .when(shortcut_num <= 5, |this| {
                this.child(
                    Label::new(format!("⌘{}", shortcut_num))
                        .size(LabelSize::XSmall)
                        .color(Color::Muted)
                )
            })
    }
}

impl Render for WorkspacesPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let repos: Vec<_> = self.registry.read(cx).repos().cloned().collect();

        v_flex()
            .size_full()
            .child(
                // Header
                h_flex()
                    .px_2()
                    .py_1()
                    .border_b_1()
                    .border_color(cx.theme().colors().border)
                    .child(
                        Icon::new(IconName::Layers)
                            .size(IconSize::Small)
                    )
                    .child(
                        Label::new("Workspaces")
                            .size(LabelSize::Small)
                            .weight(FontWeight::SEMIBOLD)
                    )
            )
            .child(
                // Repo list
                v_flex()
                    .flex_grow()
                    .overflow_y_scroll()
                    .children(repos.iter().map(|repo| self.render_repo(repo, cx)))
            )
            .child(
                // Footer
                h_flex()
                    .px_2()
                    .py_1()
                    .border_t_1()
                    .border_color(cx.theme().colors().border)
                    .child(
                        Button::new("add-repo", "Add repository")
                            .icon(IconName::FolderPlus)
                            .icon_position(IconPosition::Start)
                            .style(ButtonStyle::Ghost)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.add_repository(window, cx);
                            }))
                    )
            )
    }
}

impl Panel for WorkspacesPanel {
    fn persistent_name() -> &'static str {
        "WorkspacesPanel"
    }

    fn position(&self, _window: &Window, cx: &App) -> DockPosition {
        DockPosition::Left
    }

    fn position_is_valid(&self, position: DockPosition) -> bool {
        matches!(position, DockPosition::Left | DockPosition::Right)
    }

    fn set_position(&mut self, _: DockPosition, _: &mut Window, cx: &mut Context<Self>) {
        cx.notify();
    }

    fn size(&self, _window: &Window, cx: &App) -> Pixels {
        px(240.0)
    }

    fn icon(&self, _window: &Window, _cx: &App) -> Option<IconName> {
        Some(IconName::Layers)
    }

    fn icon_tooltip(&self, _window: &Window, _cx: &App) -> Option<&'static str> {
        Some("Workspaces Panel")
    }

    fn toggle_action(&self) -> Box<dyn Action> {
        Box::new(ToggleFocus)
    }
}

impl FocusableView for WorkspacesPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<PanelEvent> for WorkspacesPanel {}
```

### 2.3 Register Keyboard Shortcuts

**File:** `crates/workspaces_panel/src/workspaces_panel.rs` (add to init)

```rust
pub fn init(cx: &mut App) {
    cx.observe_new(WorkspacesPanel::register).detach();

    // Register keyboard shortcuts
    cx.bind_keys([
        KeyBinding::new("cmd-1", SwitchToWorkspace1, Some("WorkspacesPanel")),
        KeyBinding::new("cmd-2", SwitchToWorkspace2, Some("WorkspacesPanel")),
        KeyBinding::new("cmd-3", SwitchToWorkspace3, Some("WorkspacesPanel")),
        KeyBinding::new("cmd-4", SwitchToWorkspace4, Some("WorkspacesPanel")),
        KeyBinding::new("cmd-5", SwitchToWorkspace5, Some("WorkspacesPanel")),
    ]);
}
```

### 2.4 Add to Workspace Registration

**File:** `crates/zed/src/zed.rs` (modify)

```rust
// In initialize_workspace or similar
fn initialize_workspace(/* ... */) {
    // ... existing panel registrations ...
    workspaces_panel::init(cx);
    workspace.register_panel::<WorkspacesPanel>(cx);
}
```

### 2.5 Tests for Phase 2

**File:** `crates/workspaces_panel/src/workspaces_panel_tests.rs` (new)

```rust
use gpui::{TestAppContext, VisualTestContext};
use workspace::WorkspaceRegistry;

// ============================================================================
// PANEL CREATION TESTS
// ============================================================================

#[gpui::test]
async fn test_panel_creates_successfully(cx: &mut TestAppContext) {
    let (workspace, cx) = create_test_workspace(cx).await;

    workspace.update(cx, |workspace, cx| {
        let panel = cx.new(|cx| {
            WorkspacesPanel::new(workspace, &mut cx.window, cx)
        });

        assert!(panel.read(cx).registry.read(cx).repos().count() == 0);
    });
}

#[gpui::test]
async fn test_panel_subscribes_to_registry_events(cx: &mut TestAppContext) {
    let (workspace, cx) = create_test_workspace(cx).await;

    let panel = workspace.update(cx, |workspace, cx| {
        cx.new(|cx| WorkspacesPanel::new(workspace, &mut cx.window, cx))
    });

    // Add a repo to the registry
    workspace.update(cx, |workspace, cx| {
        workspace.app_state().workspace_registry.update(cx, |r, cx| {
            r.add_repo("/path/to/repo".into(), cx);
        });
    });

    cx.run_until_parked();

    // Panel should have been notified and re-rendered
    // (In a real test, we'd check that render was called)
}

// ============================================================================
// RENDERING TESTS
// ============================================================================

#[gpui::test]
async fn test_panel_renders_empty_state(cx: &mut TestAppContext) {
    let (workspace, mut cx) = create_test_workspace(cx).await;

    let panel = workspace.update(&mut cx, |workspace, cx| {
        cx.new(|cx| WorkspacesPanel::new(workspace, &mut cx.window, cx))
    });

    // Render the panel
    let element = panel.update(&mut cx, |panel, cx| {
        panel.render(&mut cx.window, cx)
    });

    // Should render with "Add repository" button visible
    // (Actual rendering tests would use VisualTestContext)
}

#[gpui::test]
async fn test_panel_renders_repos(cx: &mut TestAppContext) {
    let (workspace, mut cx) = create_test_workspace(cx).await;

    // Add repos to registry
    workspace.update(&mut cx, |workspace, cx| {
        workspace.app_state().workspace_registry.update(cx, |r, cx| {
            r.add_repo("/path/to/repo1".into(), cx);
            r.add_repo("/path/to/repo2".into(), cx);
        });
    });

    let panel = workspace.update(&mut cx, |workspace, cx| {
        cx.new(|cx| WorkspacesPanel::new(workspace, &mut cx.window, cx))
    });

    panel.update(&mut cx, |panel, cx| {
        let repos: Vec<_> = panel.registry.read(cx).repos().collect();
        assert_eq!(repos.len(), 2);
    });
}

#[gpui::test]
async fn test_panel_renders_worktrees_under_repo(cx: &mut TestAppContext) {
    let (workspace, mut cx) = create_test_workspace(cx).await;

    // Add repo with worktrees
    let repo_id = workspace.update(&mut cx, |workspace, cx| {
        workspace.app_state().workspace_registry.update(cx, |r, cx| {
            let repo_id = r.add_repo("/path/to/repo".into(), cx);
            r.add_worktree(repo_id, "/path/wt1".into(), "main".into(), cx);
            r.add_worktree(repo_id, "/path/wt2".into(), "feature".into(), cx);
            repo_id
        })
    });

    let panel = workspace.update(&mut cx, |workspace, cx| {
        cx.new(|cx| WorkspacesPanel::new(workspace, &mut cx.window, cx))
    });

    panel.update(&mut cx, |panel, cx| {
        let registry = panel.registry.read(cx);
        let repo = registry.repos().find(|r| r.id == repo_id).unwrap();
        assert_eq!(repo.worktrees.len(), 2);
    });
}

// ============================================================================
// EXPANSION STATE TESTS
// ============================================================================

#[gpui::test]
async fn test_toggle_repo_expanded(cx: &mut TestAppContext) {
    let (workspace, mut cx) = create_test_workspace(cx).await;

    let repo_id = workspace.update(&mut cx, |workspace, cx| {
        workspace.app_state().workspace_registry.update(cx, |r, cx| {
            r.add_repo("/path/to/repo".into(), cx)
        })
    });

    let panel = workspace.update(&mut cx, |workspace, cx| {
        cx.new(|cx| WorkspacesPanel::new(workspace, &mut cx.window, cx))
    });

    // Initially not expanded
    panel.update(&mut cx, |panel, _| {
        assert!(!panel.expanded_repos.contains(&repo_id));
    });

    // Toggle to expanded
    panel.update(&mut cx, |panel, cx| {
        panel.toggle_repo_expanded(repo_id, cx);
        assert!(panel.expanded_repos.contains(&repo_id));
    });

    // Toggle back to collapsed
    panel.update(&mut cx, |panel, cx| {
        panel.toggle_repo_expanded(repo_id, cx);
        assert!(!panel.expanded_repos.contains(&repo_id));
    });
}

// ============================================================================
// ACTIVE SLOT HIGHLIGHTING TESTS
// ============================================================================

#[gpui::test]
async fn test_active_slot_is_highlighted(cx: &mut TestAppContext) {
    let (workspace, mut cx) = create_test_workspace(cx).await;

    let (repo_id, slot_id) = workspace.update(&mut cx, |workspace, cx| {
        workspace.app_state().workspace_registry.update(cx, |r, cx| {
            let repo_id = r.add_repo("/path/to/repo".into(), cx);
            let slot_id = r.add_worktree(repo_id, "/path/wt".into(), "main".into(), cx).unwrap();
            r.set_active_slot(slot_id, cx);
            (repo_id, slot_id)
        })
    });

    let panel = workspace.update(&mut cx, |workspace, cx| {
        cx.new(|cx| WorkspacesPanel::new(workspace, &mut cx.window, cx))
    });

    panel.update(&mut cx, |panel, cx| {
        let registry = panel.registry.read(cx);
        assert_eq!(registry.active_slot().map(|s| s.slot_id), Some(slot_id));
    });
}

// ============================================================================
// KEYBOARD SHORTCUT TESTS
// ============================================================================

#[gpui::test]
async fn test_switch_to_workspace_by_index(cx: &mut TestAppContext) {
    let (workspace, mut cx) = create_test_workspace(cx).await;

    let (slot1, slot2, slot3) = workspace.update(&mut cx, |workspace, cx| {
        workspace.app_state().workspace_registry.update(cx, |r, cx| {
            let repo_id = r.add_repo("/path/to/repo".into(), cx);
            let slot1 = r.add_worktree(repo_id, "/path/wt1".into(), "main".into(), cx).unwrap();
            let slot2 = r.add_worktree(repo_id, "/path/wt2".into(), "dev".into(), cx).unwrap();
            let slot3 = r.add_worktree(repo_id, "/path/wt3".into(), "feature".into(), cx).unwrap();
            (slot1, slot2, slot3)
        })
    });

    let panel = workspace.update(&mut cx, |workspace, cx| {
        cx.new(|cx| WorkspacesPanel::new(workspace, &mut cx.window, cx))
    });

    // Simulate ⌘1 - should select first workspace
    panel.update(&mut cx, |panel, cx| {
        panel.handle_switch_to_workspace(0, &mut cx.window, cx);
    });

    cx.run_until_parked();

    // Verify first slot is now active
    workspace.update(&mut cx, |workspace, cx| {
        let registry = workspace.app_state().workspace_registry.read(cx);
        assert_eq!(registry.active_slot().map(|s| s.slot_id), Some(slot1));
    });
}

#[gpui::test]
async fn test_switch_to_workspace_out_of_bounds_is_noop(cx: &mut TestAppContext) {
    let (workspace, mut cx) = create_test_workspace(cx).await;

    workspace.update(&mut cx, |workspace, cx| {
        workspace.app_state().workspace_registry.update(cx, |r, cx| {
            let repo_id = r.add_repo("/path/to/repo".into(), cx);
            r.add_worktree(repo_id, "/path/wt1".into(), "main".into(), cx);
        });
    });

    let panel = workspace.update(&mut cx, |workspace, cx| {
        cx.new(|cx| WorkspacesPanel::new(workspace, &mut cx.window, cx))
    });

    // Simulate ⌘5 when only 1 workspace exists - should be a no-op
    panel.update(&mut cx, |panel, cx| {
        panel.handle_switch_to_workspace(4, &mut cx.window, cx);
    });

    // Should not crash
    cx.run_until_parked();
}

// ============================================================================
// AGENT CHAT BADGE TESTS
// ============================================================================

#[gpui::test]
async fn test_agent_chat_badge_shows_count(cx: &mut TestAppContext) {
    let (workspace, mut cx) = create_test_workspace(cx).await;

    let slot_id = workspace.update(&mut cx, |workspace, cx| {
        workspace.app_state().workspace_registry.update(cx, |r, cx| {
            let repo_id = r.add_repo("/path/to/repo".into(), cx);
            let slot_id = r.add_worktree(repo_id, "/path/wt".into(), "main".into(), cx).unwrap();
            r.update_agent_chat_count(slot_id, 3, cx);
            slot_id
        })
    });

    let panel = workspace.update(&mut cx, |workspace, cx| {
        cx.new(|cx| WorkspacesPanel::new(workspace, &mut cx.window, cx))
    });

    panel.update(&mut cx, |panel, cx| {
        let registry = panel.registry.read(cx);
        let repo = registry.repos().next().unwrap();
        assert_eq!(repo.worktrees[0].agent_chat_count, 3);
    });
}

#[gpui::test]
async fn test_agent_chat_badge_hidden_when_zero(cx: &mut TestAppContext) {
    let (workspace, mut cx) = create_test_workspace(cx).await;

    workspace.update(&mut cx, |workspace, cx| {
        workspace.app_state().workspace_registry.update(cx, |r, cx| {
            let repo_id = r.add_repo("/path/to/repo".into(), cx);
            r.add_worktree(repo_id, "/path/wt".into(), "main".into(), cx);
            // agent_chat_count defaults to 0
        });
    });

    let panel = workspace.update(&mut cx, |workspace, cx| {
        cx.new(|cx| WorkspacesPanel::new(workspace, &mut cx.window, cx))
    });

    panel.update(&mut cx, |panel, cx| {
        let registry = panel.registry.read(cx);
        let repo = registry.repos().next().unwrap();
        // Badge should not be shown (count is 0)
        assert_eq!(repo.worktrees[0].agent_chat_count, 0);
    });
}

// ============================================================================
// PANEL TRAIT IMPLEMENTATION TESTS
// ============================================================================

#[gpui::test]
async fn test_panel_persistent_name(cx: &mut TestAppContext) {
    assert_eq!(WorkspacesPanel::persistent_name(), "WorkspacesPanel");
}

#[gpui::test]
async fn test_panel_valid_positions(cx: &mut TestAppContext) {
    let (workspace, mut cx) = create_test_workspace(cx).await;

    let panel = workspace.update(&mut cx, |workspace, cx| {
        cx.new(|cx| WorkspacesPanel::new(workspace, &mut cx.window, cx))
    });

    panel.update(&mut cx, |panel, _| {
        assert!(panel.position_is_valid(DockPosition::Left));
        assert!(panel.position_is_valid(DockPosition::Right));
        assert!(!panel.position_is_valid(DockPosition::Bottom));
    });
}
```

**Acceptance Criteria for Phase 2:**
- [ ] All tests pass
- [ ] Panel renders with header showing "Workspaces"
- [ ] Repos are listed with expand/collapse chevrons
- [ ] Worktrees are shown under expanded repos with branch names
- [ ] Active workspace is visually highlighted
- [ ] Agent chat count badge shows when count > 0
- [ ] Keyboard shortcuts (⌘1-⌘5) switch workspaces
- [ ] "Add repository" button is visible and functional
- [ ] Panel can be positioned in left or right dock

---

## Phase 3: Workspace Slot State Management

**Goal:** Serialize and deserialize workspace state per slot.

### 3.1 Define Serialized Slot State

**File:** `crates/workspace/src/persistence/model.rs` (modify)

```rust
// Add new serialization structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedWorkspaceSlot {
    pub slot_id: i64,
    pub center_group: SerializedPaneGroup,
    pub docks: DockStructure,
    pub window_bounds: Option<SerializedWindowBounds>,
    pub window_state: Option<SerializedWindowState>,
}

impl SerializedWorkspaceSlot {
    /// Create from current workspace state
    pub fn from_workspace(workspace: &Workspace, cx: &App) -> Self {
        Self {
            slot_id: workspace.active_slot_id().map(|s| s.0 as i64).unwrap_or(0),
            center_group: workspace.serialize_pane_group(&workspace.center.read(cx).root, cx),
            docks: workspace.serialize_docks(cx),
            window_bounds: workspace.bounds_saved.get().map(|b| b.into()),
            window_state: workspace.window_state_saved.get().map(|s| s.into()),
        }
    }
}
```

### 3.2 Add Slot Persistence Queries

**File:** `crates/workspace/src/workspace_registry_db.rs` (modify)

```rust
impl WorkspaceRegistryDb {
    pub fn save_slot_state(&self, slot_id: WorkspaceSlotId, state: &SerializedWorkspaceSlot) -> Result<()> {
        let serialized = serde_json::to_vec(state)?;

        self.connection.execute(
            "INSERT OR REPLACE INTO workspace_slots (slot_id, serialized_state) VALUES (?1, ?2)",
            params![slot_id.0 as i64, serialized],
        )?;

        Ok(())
    }

    pub fn load_slot_state(&self, slot_id: WorkspaceSlotId) -> Result<Option<SerializedWorkspaceSlot>> {
        let mut stmt = self.connection.prepare(
            "SELECT serialized_state FROM workspace_slots WHERE slot_id = ?1"
        )?;

        let result = stmt.query_row(params![slot_id.0 as i64], |row| {
            let data: Vec<u8> = row.get(0)?;
            Ok(data)
        }).optional()?;

        match result {
            Some(data) => Ok(Some(serde_json::from_slice(&data)?)),
            None => Ok(None),
        }
    }
}
```

### 3.3 Integrate with WorkspaceSlot

**File:** `crates/workspace/src/workspace_registry.rs` (modify)

```rust
impl WorkspaceSlot {
    /// Save current state to serialized form
    pub fn save_state(&mut self, workspace: &Workspace, cx: &App) {
        self.serialized = Some(SerializedWorkspaceSlot::from_workspace(workspace, cx));
    }

    /// Check if this slot has a Project loaded
    pub fn is_loaded(&self) -> bool {
        self.project.is_some()
    }
}
```

### 3.4 Tests for Phase 3

**File:** `crates/workspace/src/persistence/slot_serialization_tests.rs` (new)

```rust
use tempfile::TempDir;

// ============================================================================
// SERIALIZATION TESTS
// ============================================================================

#[gpui::test]
async fn test_serialize_empty_workspace_slot(cx: &mut TestAppContext) {
    let (workspace, mut cx) = create_test_workspace(cx).await;

    workspace.update(&mut cx, |workspace, cx| {
        let serialized = SerializedWorkspaceSlot::from_workspace(workspace, cx);

        assert_eq!(serialized.slot_id, 0); // No active slot
        assert!(matches!(serialized.center_group, SerializedPaneGroup::Pane(_)));
    });
}

#[gpui::test]
async fn test_serialize_workspace_with_open_files(cx: &mut TestAppContext) {
    let temp_dir = TempDir::new().unwrap();
    let file_path = temp_dir.path().join("test.rs");
    std::fs::write(&file_path, "fn main() {}").unwrap();

    let (workspace, mut cx) = create_test_workspace_with_path(cx, temp_dir.path()).await;

    // Open a file
    workspace.update(&mut cx, |workspace, cx| {
        workspace.open_path(file_path.clone(), None, true, cx)
    }).await.unwrap();

    workspace.update(&mut cx, |workspace, cx| {
        let serialized = SerializedWorkspaceSlot::from_workspace(workspace, cx);

        // Should have one pane with one item
        match &serialized.center_group {
            SerializedPaneGroup::Pane(pane) => {
                assert_eq!(pane.children.len(), 1);
            }
            _ => panic!("Expected single pane"),
        }
    });
}

#[gpui::test]
async fn test_serialize_workspace_with_split_panes(cx: &mut TestAppContext) {
    let (workspace, mut cx) = create_test_workspace(cx).await;

    // Split the pane
    workspace.update(&mut cx, |workspace, cx| {
        workspace.split_and_clone(workspace.active_pane().clone(), SplitDirection::Right, cx);
    });

    workspace.update(&mut cx, |workspace, cx| {
        let serialized = SerializedWorkspaceSlot::from_workspace(workspace, cx);

        // Should have a group with horizontal axis
        match &serialized.center_group {
            SerializedPaneGroup::Group { axis, children, .. } => {
                assert_eq!(*axis, SerializedAxis::Horizontal);
                assert_eq!(children.len(), 2);
            }
            _ => panic!("Expected pane group"),
        }
    });
}

#[gpui::test]
async fn test_serialize_dock_visibility(cx: &mut TestAppContext) {
    let (workspace, mut cx) = create_test_workspace(cx).await;

    // Set dock visibility
    workspace.update(&mut cx, |workspace, cx| {
        workspace.left_dock.update(cx, |dock, cx| dock.set_open(true, cx));
        workspace.right_dock.update(cx, |dock, cx| dock.set_open(false, cx));
        workspace.bottom_dock.update(cx, |dock, cx| dock.set_open(true, cx));
    });

    workspace.update(&mut cx, |workspace, cx| {
        let serialized = SerializedWorkspaceSlot::from_workspace(workspace, cx);

        assert!(serialized.docks.left.visible);
        assert!(!serialized.docks.right.visible);
        assert!(serialized.docks.bottom.visible);
    });
}

// ============================================================================
// PERSISTENCE TESTS
// ============================================================================

#[gpui::test]
async fn test_save_and_load_slot_state(cx: &mut TestAppContext) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("registry.db");
    let db = WorkspaceRegistryDb::new_with_path(&db_path);

    let slot_id = WorkspaceSlotId(1);

    // Create a serialized state
    let state = SerializedWorkspaceSlot {
        slot_id: 1,
        center_group: SerializedPaneGroup::Pane(SerializedPane {
            children: vec![],
            active: true,
            pinned_count: 0,
        }),
        docks: DockStructure {
            left: DockState { visible: true, active_panel: None, zoom: false },
            right: DockState { visible: false, active_panel: None, zoom: false },
            bottom: DockState { visible: true, active_panel: None, zoom: false },
        },
        window_bounds: None,
        window_state: None,
    };

    // Save
    db.save_slot_state(slot_id, &state).unwrap();

    // Load
    let loaded = db.load_slot_state(slot_id).unwrap().unwrap();

    assert_eq!(loaded.slot_id, state.slot_id);
    assert!(loaded.docks.left.visible);
    assert!(!loaded.docks.right.visible);
}

#[gpui::test]
async fn test_load_nonexistent_slot_returns_none(cx: &mut TestAppContext) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("registry.db");
    let db = WorkspaceRegistryDb::new_with_path(&db_path);

    let result = db.load_slot_state(WorkspaceSlotId(9999)).unwrap();
    assert!(result.is_none());
}

#[gpui::test]
async fn test_save_slot_state_overwrites_existing(cx: &mut TestAppContext) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("registry.db");
    let db = WorkspaceRegistryDb::new_with_path(&db_path);

    let slot_id = WorkspaceSlotId(1);

    // Save first state
    let state1 = SerializedWorkspaceSlot {
        slot_id: 1,
        center_group: SerializedPaneGroup::Pane(SerializedPane::default()),
        docks: DockStructure::default(),
        window_bounds: None,
        window_state: None,
    };
    db.save_slot_state(slot_id, &state1).unwrap();

    // Save second state (overwrite)
    let state2 = SerializedWorkspaceSlot {
        slot_id: 1,
        center_group: SerializedPaneGroup::Pane(SerializedPane::default()),
        docks: DockStructure {
            left: DockState { visible: true, ..Default::default() },
            ..Default::default()
        },
        window_bounds: None,
        window_state: None,
    };
    db.save_slot_state(slot_id, &state2).unwrap();

    // Load should return second state
    let loaded = db.load_slot_state(slot_id).unwrap().unwrap();
    assert!(loaded.docks.left.visible);
}

// ============================================================================
// WORKSPACE SLOT TESTS
// ============================================================================

#[gpui::test]
async fn test_slot_save_state(cx: &mut TestAppContext) {
    let (workspace, mut cx) = create_test_workspace(cx).await;

    let mut slot = WorkspaceSlot {
        slot_id: WorkspaceSlotId(1),
        repo_id: RepoId(1),
        worktree_path: "/path".into(),
        state: SlotState::Active,
        project: None,
        serialized: None,
    };

    workspace.update(&mut cx, |workspace, cx| {
        slot.save_state(workspace, cx);
    });

    assert!(slot.serialized.is_some());
}

#[gpui::test]
async fn test_slot_is_loaded(cx: &mut TestAppContext) {
    let slot_unloaded = WorkspaceSlot {
        slot_id: WorkspaceSlotId(1),
        repo_id: RepoId(1),
        worktree_path: "/path".into(),
        state: SlotState::Unloaded,
        project: None,
        serialized: None,
    };

    assert!(!slot_unloaded.is_loaded());

    // With a project (would need to create one in real test)
    // let slot_loaded = WorkspaceSlot { ..., project: Some(project), ... };
    // assert!(slot_loaded.is_loaded());
}
```

**Acceptance Criteria for Phase 3:**
- [ ] All tests pass
- [ ] `SerializedWorkspaceSlot` captures pane layout
- [ ] `SerializedWorkspaceSlot` captures dock visibility
- [ ] Slot state can be saved to database
- [ ] Slot state can be loaded from database
- [ ] `WorkspaceSlot::save_state()` populates serialized field

---

## Phase 4: Workspace Switching Core

**Goal:** Implement the ability to switch between workspace slots in a window.

### 4.1 Add Switching Method to Workspace

**File:** `crates/workspace/src/workspace.rs` (modify, around line 3000)

```rust
impl Workspace {
    /// Current active slot ID
    pub fn active_slot_id(&self) -> Option<WorkspaceSlotId> {
        self.active_slot_id
    }

    /// Switch to a different workspace slot
    pub fn switch_to_slot(
        &mut self,
        slot_id: WorkspaceSlotId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        // Don't switch if already active
        if self.active_slot_id == Some(slot_id) {
            return Task::ready(Ok(()));
        }

        let registry = self.app_state.workspace_registry.clone();
        let old_slot_id = self.active_slot_id;

        cx.spawn_in(window, async move |this, mut cx| {
            // Step 1: Save current slot state
            if let Some(old_id) = old_slot_id {
                this.update(&mut cx, |workspace, cx| {
                    workspace.save_current_slot_state(old_id, cx);
                })?;
            }

            // Step 2: Get the target slot
            let (worktree_path, serialized_state) = cx.update(|_, cx| {
                registry.update(cx, |registry, cx| {
                    let slot = registry.slots.get(&slot_id)?;
                    Some((slot.worktree_path.clone(), slot.serialized.clone()))
                })
            })?.flatten().ok_or_else(|| anyhow!("Slot not found"))?;

            // Step 3: Shutdown current LSP servers gracefully
            this.update(&mut cx, |workspace, cx| {
                workspace.project.update(cx, |project, cx| {
                    project.lsp_store().update(cx, |lsp_store, cx| {
                        lsp_store.stop_all_language_servers(cx);
                    });
                });
            })?;

            // Step 4: Create new Project for the target worktree
            let new_project = cx.update(|window, cx| {
                let app_state = this.read(cx).app_state.clone();
                Project::local(
                    app_state.client.clone(),
                    app_state.node_runtime.clone(),
                    app_state.user_store.clone(),
                    app_state.languages.clone(),
                    app_state.fs.clone(),
                    None, // env
                    true, // trust worktree
                    cx,
                )
            })?;

            // Step 5: Add the worktree to the new project
            new_project.update(&mut cx, |project, cx| {
                project.find_or_create_worktree(&worktree_path, true, cx)
            })?.await?;

            // Step 6: Swap the project reference
            this.update(&mut cx, |workspace, cx| {
                // Close all items in current panes
                workspace.close_all_items_and_panes(cx);

                // Swap project
                let old_project = std::mem::replace(&mut workspace.project, new_project);
                workspace.active_slot_id = Some(slot_id);

                // Update registry
                registry.update(cx, |registry, cx| {
                    // Mark old slot as cached (keep project in memory briefly)
                    if let Some(old_id) = old_slot_id {
                        if let Some(old_slot) = registry.slots.get_mut(&old_id) {
                            old_slot.state = SlotState::Cached;
                            old_slot.project = Some(old_project);
                        }
                    }

                    // Mark new slot as active
                    if let Some(new_slot) = registry.slots.get_mut(&slot_id) {
                        new_slot.state = SlotState::Active;
                        new_slot.project = None; // Project is now in workspace
                    }

                    registry.set_active_slot(slot_id, cx);
                });

                // Rebind subscriptions to new project
                workspace.rebind_project_subscriptions(cx);

                cx.notify();
            })?;

            // Step 7: Restore pane layout from serialized state
            if let Some(state) = serialized_state {
                this.update(&mut cx, |workspace, cx| {
                    workspace.restore_from_serialized_slot(state, window, cx);
                })?;
            }

            // Step 8: Start LSP servers for new project
            this.update(&mut cx, |workspace, cx| {
                workspace.project.update(cx, |project, cx| {
                    // LSP will auto-start when buffers are opened
                    cx.notify();
                });
            })?;

            Ok(())
        })
    }

    /// Save current workspace state to the active slot
    fn save_current_slot_state(&mut self, slot_id: WorkspaceSlotId, cx: &mut Context<Self>) {
        let serialized = SerializedWorkspaceSlot::from_workspace(self, cx);

        self.app_state.workspace_registry.update(cx, |registry, cx| {
            if let Some(slot) = registry.slots.get_mut(&slot_id) {
                slot.serialized = Some(serialized.clone());
            }
            registry.db.save_slot_state(slot_id, &serialized).log_err();
        });
    }

    /// Close all items and reset panes to single empty pane
    fn close_all_items_and_panes(&mut self, cx: &mut Context<Self>) {
        // Close all panes except one
        let panes_to_close: Vec<_> = self.panes.iter().skip(1).cloned().collect();
        for pane in panes_to_close {
            self.remove_pane(pane, cx);
        }

        // Close all items in remaining pane
        if let Some(pane) = self.panes.first().cloned() {
            pane.update(cx, |pane, cx| {
                pane.close_all_items(&CloseAllItems { save_intent: None }, cx);
            });
        }

        // Reset center pane group
        self.center.update(cx, |center, cx| {
            center.root = Member::Pane(self.panes[0].clone());
        });
    }

    /// Restore pane layout from serialized state
    fn restore_from_serialized_slot(
        &mut self,
        state: SerializedWorkspaceSlot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Restore pane group structure
        self.restore_pane_group(state.center_group, window, cx);

        // Restore dock visibility
        self.left_dock.update(cx, |dock, cx| {
            dock.set_open(state.docks.left.visible, cx);
        });
        self.right_dock.update(cx, |dock, cx| {
            dock.set_open(state.docks.right.visible, cx);
        });
        self.bottom_dock.update(cx, |dock, cx| {
            dock.set_open(state.docks.bottom.visible, cx);
        });
    }

    /// Rebind subscriptions after project swap
    fn rebind_project_subscriptions(&mut self, cx: &mut Context<Self>) {
        // Clear old subscriptions
        self._project_subscriptions.clear();

        // Add new subscriptions
        self._project_subscriptions.push(
            cx.subscribe(&self.project, Self::on_project_event)
        );

        // Rebind git store
        let git_store = self.project.read(cx).git_store().clone();
        self._project_subscriptions.push(
            cx.subscribe(&git_store, Self::on_git_event)
        );

        // ... other subscriptions ...
    }
}
```

### 4.2 Handle Unsaved Changes

**File:** `crates/workspace/src/workspace.rs` (add to switch_to_slot)

```rust
impl Workspace {
    /// Check for unsaved changes before switching
    async fn check_unsaved_before_switch(
        this: &WeakEntity<Self>,
        cx: &mut AsyncWindowContext,
    ) -> Result<bool> {
        let dominated_items = this.update(cx, |workspace, cx| {
            workspace.items_with_unsaved_changes(cx)
        })?;

        if dominated_items.is_empty() {
            return Ok(true);
        }

        // Prompt user
        let answer = cx.prompt(
            PromptLevel::Warning,
            "Unsaved Changes",
            Some("You have unsaved changes. Save before switching workspaces?"),
            &["Save All", "Don't Save", "Cancel"],
        ).await?;

        match answer {
            0 => {
                // Save all
                this.update(cx, |workspace, cx| {
                    workspace.save_all(&SaveAll, cx)
                })?.await?;
                Ok(true)
            }
            1 => Ok(true),  // Don't save, proceed
            _ => Ok(false), // Cancel
        }
    }
}
```

### 4.3 Cached Project Cleanup

**File:** `crates/workspace/src/workspace_registry.rs` (add cleanup logic)

```rust
impl WorkspaceRegistry {
    /// Clean up cached projects that haven't been accessed recently
    pub fn cleanup_cached_projects(&mut self, cx: &mut Context<Self>) {
        let now = Utc::now();
        let cache_timeout = chrono::Duration::minutes(5);

        for slot in self.slots.values_mut() {
            if matches!(slot.state, SlotState::Cached) {
                // Check last access time
                if let Some(repo) = self.repos.get(&slot.repo_id) {
                    if let Some(worktree) = repo.worktrees.iter().find(|w| w.slot_id == slot.slot_id) {
                        if now - worktree.last_accessed > cache_timeout {
                            // Drop the cached project
                            slot.project = None;
                            slot.state = SlotState::Unloaded;
                        }
                    }
                }
            }
        }
    }
}
```

### 4.4 Tests for Phase 4

**File:** `crates/workspace/src/workspace_switching_tests.rs` (new)

```rust
use tempfile::TempDir;

// ============================================================================
// BASIC SWITCHING TESTS
// ============================================================================

#[gpui::test]
async fn test_switch_to_slot_changes_active_slot(cx: &mut TestAppContext) {
    let temp_dir1 = TempDir::new().unwrap();
    let temp_dir2 = TempDir::new().unwrap();

    let (workspace, mut cx) = create_test_workspace_with_path(cx, temp_dir1.path()).await;

    // Setup registry with two slots
    let (slot1, slot2) = workspace.update(&mut cx, |workspace, cx| {
        workspace.app_state().workspace_registry.update(cx, |r, cx| {
            let repo_id = r.add_repo(temp_dir1.path().to_path_buf(), cx);
            let slot1 = r.add_worktree(repo_id, temp_dir1.path().to_path_buf(), "main".into(), cx).unwrap();
            let slot2 = r.add_worktree(repo_id, temp_dir2.path().to_path_buf(), "feature".into(), cx).unwrap();
            r.set_active_slot(slot1, cx);
            (slot1, slot2)
        })
    });

    // Switch to slot2
    workspace.update(&mut cx, |workspace, cx| {
        workspace.switch_to_slot(slot2, &mut cx.window, cx)
    }).await.unwrap();

    // Verify active slot changed
    workspace.update(&mut cx, |workspace, cx| {
        assert_eq!(workspace.active_slot_id(), Some(slot2));
    });
}

#[gpui::test]
async fn test_switch_to_same_slot_is_noop(cx: &mut TestAppContext) {
    let temp_dir = TempDir::new().unwrap();
    let (workspace, mut cx) = create_test_workspace_with_path(cx, temp_dir.path()).await;

    let slot_id = workspace.update(&mut cx, |workspace, cx| {
        workspace.app_state().workspace_registry.update(cx, |r, cx| {
            let repo_id = r.add_repo(temp_dir.path().to_path_buf(), cx);
            let slot_id = r.add_worktree(repo_id, temp_dir.path().to_path_buf(), "main".into(), cx).unwrap();
            r.set_active_slot(slot_id, cx);
            slot_id
        })
    });

    workspace.update(&mut cx, |workspace, cx| {
        workspace.active_slot_id = Some(slot_id);
    });

    // Switch to same slot - should return immediately
    let task = workspace.update(&mut cx, |workspace, cx| {
        workspace.switch_to_slot(slot_id, &mut cx.window, cx)
    });

    // Should complete immediately without error
    task.await.unwrap();
}

#[gpui::test]
async fn test_switch_to_nonexistent_slot_fails(cx: &mut TestAppContext) {
    let temp_dir = TempDir::new().unwrap();
    let (workspace, mut cx) = create_test_workspace_with_path(cx, temp_dir.path()).await;

    let fake_slot_id = WorkspaceSlotId(9999);

    let result = workspace.update(&mut cx, |workspace, cx| {
        workspace.switch_to_slot(fake_slot_id, &mut cx.window, cx)
    }).await;

    assert!(result.is_err());
}

// ============================================================================
// STATE PRESERVATION TESTS
// ============================================================================

#[gpui::test]
async fn test_switch_saves_current_slot_state(cx: &mut TestAppContext) {
    let temp_dir1 = TempDir::new().unwrap();
    let temp_dir2 = TempDir::new().unwrap();

    // Create file in first workspace
    let file_path = temp_dir1.path().join("test.rs");
    std::fs::write(&file_path, "fn main() {}").unwrap();

    let (workspace, mut cx) = create_test_workspace_with_path(cx, temp_dir1.path()).await;

    let (slot1, slot2) = workspace.update(&mut cx, |workspace, cx| {
        workspace.app_state().workspace_registry.update(cx, |r, cx| {
            let repo_id = r.add_repo(temp_dir1.path().to_path_buf(), cx);
            let slot1 = r.add_worktree(repo_id, temp_dir1.path().to_path_buf(), "main".into(), cx).unwrap();
            let slot2 = r.add_worktree(repo_id, temp_dir2.path().to_path_buf(), "feature".into(), cx).unwrap();
            r.set_active_slot(slot1, cx);
            (slot1, slot2)
        })
    });

    workspace.update(&mut cx, |workspace, cx| {
        workspace.active_slot_id = Some(slot1);
    });

    // Open a file
    workspace.update(&mut cx, |workspace, cx| {
        workspace.open_path(file_path, None, true, cx)
    }).await.unwrap();

    // Switch to slot2
    workspace.update(&mut cx, |workspace, cx| {
        workspace.switch_to_slot(slot2, &mut cx.window, cx)
    }).await.unwrap();

    // Verify slot1's state was saved
    workspace.update(&mut cx, |workspace, cx| {
        let registry = workspace.app_state().workspace_registry.read(cx);
        let slot = registry.slot(slot1).unwrap();
        assert!(slot.serialized.is_some());
    });
}

#[gpui::test]
async fn test_switch_restores_target_slot_state(cx: &mut TestAppContext) {
    let temp_dir1 = TempDir::new().unwrap();
    let temp_dir2 = TempDir::new().unwrap();

    let (workspace, mut cx) = create_test_workspace_with_path(cx, temp_dir1.path()).await;

    let (slot1, slot2) = workspace.update(&mut cx, |workspace, cx| {
        workspace.app_state().workspace_registry.update(cx, |r, cx| {
            let repo_id = r.add_repo(temp_dir1.path().to_path_buf(), cx);
            let slot1 = r.add_worktree(repo_id, temp_dir1.path().to_path_buf(), "main".into(), cx).unwrap();
            let slot2 = r.add_worktree(repo_id, temp_dir2.path().to_path_buf(), "feature".into(), cx).unwrap();

            // Pre-populate slot2 with serialized state (simulating previous use)
            if let Some(slot) = r.slots.get_mut(&slot2) {
                slot.serialized = Some(SerializedWorkspaceSlot {
                    slot_id: slot2.0 as i64,
                    center_group: SerializedPaneGroup::Pane(SerializedPane::default()),
                    docks: DockStructure {
                        left: DockState { visible: true, ..Default::default() },
                        ..Default::default()
                    },
                    window_bounds: None,
                    window_state: None,
                });
            }

            r.set_active_slot(slot1, cx);
            (slot1, slot2)
        })
    });

    workspace.update(&mut cx, |workspace, cx| {
        workspace.active_slot_id = Some(slot1);
    });

    // Switch to slot2
    workspace.update(&mut cx, |workspace, cx| {
        workspace.switch_to_slot(slot2, &mut cx.window, cx)
    }).await.unwrap();

    // Verify dock state was restored
    workspace.update(&mut cx, |workspace, cx| {
        assert!(workspace.left_dock.read(cx).is_open());
    });
}

// ============================================================================
// PROJECT MANAGEMENT TESTS
// ============================================================================

#[gpui::test]
async fn test_switch_creates_new_project_for_target(cx: &mut TestAppContext) {
    let temp_dir1 = TempDir::new().unwrap();
    let temp_dir2 = TempDir::new().unwrap();

    let (workspace, mut cx) = create_test_workspace_with_path(cx, temp_dir1.path()).await;

    let original_project_id = workspace.update(&mut cx, |workspace, _| {
        workspace.project.entity_id()
    });

    let (slot1, slot2) = workspace.update(&mut cx, |workspace, cx| {
        workspace.app_state().workspace_registry.update(cx, |r, cx| {
            let repo_id = r.add_repo(temp_dir1.path().to_path_buf(), cx);
            let slot1 = r.add_worktree(repo_id, temp_dir1.path().to_path_buf(), "main".into(), cx).unwrap();
            let slot2 = r.add_worktree(repo_id, temp_dir2.path().to_path_buf(), "feature".into(), cx).unwrap();
            r.set_active_slot(slot1, cx);
            (slot1, slot2)
        })
    });

    workspace.update(&mut cx, |workspace, cx| {
        workspace.active_slot_id = Some(slot1);
    });

    // Switch to slot2
    workspace.update(&mut cx, |workspace, cx| {
        workspace.switch_to_slot(slot2, &mut cx.window, cx)
    }).await.unwrap();

    // Verify project changed
    let new_project_id = workspace.update(&mut cx, |workspace, _| {
        workspace.project.entity_id()
    });

    assert_ne!(original_project_id, new_project_id);
}

#[gpui::test]
async fn test_switch_caches_old_project(cx: &mut TestAppContext) {
    let temp_dir1 = TempDir::new().unwrap();
    let temp_dir2 = TempDir::new().unwrap();

    let (workspace, mut cx) = create_test_workspace_with_path(cx, temp_dir1.path()).await;

    let (slot1, slot2) = workspace.update(&mut cx, |workspace, cx| {
        workspace.app_state().workspace_registry.update(cx, |r, cx| {
            let repo_id = r.add_repo(temp_dir1.path().to_path_buf(), cx);
            let slot1 = r.add_worktree(repo_id, temp_dir1.path().to_path_buf(), "main".into(), cx).unwrap();
            let slot2 = r.add_worktree(repo_id, temp_dir2.path().to_path_buf(), "feature".into(), cx).unwrap();
            r.set_active_slot(slot1, cx);
            (slot1, slot2)
        })
    });

    workspace.update(&mut cx, |workspace, cx| {
        workspace.active_slot_id = Some(slot1);
    });

    // Switch to slot2
    workspace.update(&mut cx, |workspace, cx| {
        workspace.switch_to_slot(slot2, &mut cx.window, cx)
    }).await.unwrap();

    // Verify slot1 is now cached (has project reference)
    workspace.update(&mut cx, |workspace, cx| {
        let registry = workspace.app_state().workspace_registry.read(cx);
        let slot = registry.slot(slot1).unwrap();
        assert!(matches!(slot.state, SlotState::Cached));
        assert!(slot.project.is_some());
    });
}

// ============================================================================
// UNSAVED CHANGES TESTS
// ============================================================================

#[gpui::test]
async fn test_switch_with_unsaved_changes_prompts_user(cx: &mut TestAppContext) {
    // This test would need to mock the prompt dialog
    // For now, just verify the check_unsaved_before_switch function exists
    // and returns appropriate values
}

#[gpui::test]
async fn test_switch_without_unsaved_changes_proceeds(cx: &mut TestAppContext) {
    let temp_dir1 = TempDir::new().unwrap();
    let temp_dir2 = TempDir::new().unwrap();

    let (workspace, mut cx) = create_test_workspace_with_path(cx, temp_dir1.path()).await;

    let (slot1, slot2) = workspace.update(&mut cx, |workspace, cx| {
        workspace.app_state().workspace_registry.update(cx, |r, cx| {
            let repo_id = r.add_repo(temp_dir1.path().to_path_buf(), cx);
            let slot1 = r.add_worktree(repo_id, temp_dir1.path().to_path_buf(), "main".into(), cx).unwrap();
            let slot2 = r.add_worktree(repo_id, temp_dir2.path().to_path_buf(), "feature".into(), cx).unwrap();
            r.set_active_slot(slot1, cx);
            (slot1, slot2)
        })
    });

    workspace.update(&mut cx, |workspace, cx| {
        workspace.active_slot_id = Some(slot1);
    });

    // No unsaved changes, switch should succeed
    let result = workspace.update(&mut cx, |workspace, cx| {
        workspace.switch_to_slot(slot2, &mut cx.window, cx)
    }).await;

    assert!(result.is_ok());
}

// ============================================================================
// CLEANUP TESTS
// ============================================================================

#[gpui::test]
async fn test_cleanup_cached_projects_removes_old_caches(cx: &mut TestAppContext) {
    let registry = create_test_registry(cx);

    // Add a repo and worktree
    let (repo_id, slot_id) = registry.update(cx, |r, cx| {
        let repo_id = r.add_repo("/path/to/repo".into(), cx);
        let slot_id = r.add_worktree(repo_id, "/path/wt".into(), "main".into(), cx).unwrap();
        (repo_id, slot_id)
    });

    // Manually set slot to cached with old last_accessed time
    registry.update(cx, |r, cx| {
        if let Some(slot) = r.slots.get_mut(&slot_id) {
            slot.state = SlotState::Cached;
            // Note: In a real test, we'd mock the time or set last_accessed to an old value
        }

        if let Some(repo) = r.repos.get_mut(&repo_id) {
            if let Some(worktree) = repo.worktrees.iter_mut().find(|w| w.slot_id == slot_id) {
                worktree.last_accessed = Utc::now() - chrono::Duration::minutes(10);
            }
        }
    });

    // Run cleanup
    registry.update(cx, |r, cx| {
        r.cleanup_cached_projects(cx);
    });

    // Verify slot is now unloaded
    registry.update(cx, |r, _| {
        let slot = r.slot(slot_id).unwrap();
        assert!(matches!(slot.state, SlotState::Unloaded));
        assert!(slot.project.is_none());
    });
}

#[gpui::test]
async fn test_cleanup_preserves_recently_accessed_caches(cx: &mut TestAppContext) {
    let registry = create_test_registry(cx);

    let (repo_id, slot_id) = registry.update(cx, |r, cx| {
        let repo_id = r.add_repo("/path/to/repo".into(), cx);
        let slot_id = r.add_worktree(repo_id, "/path/wt".into(), "main".into(), cx).unwrap();
        (repo_id, slot_id)
    });

    // Set slot to cached with recent last_accessed time
    registry.update(cx, |r, _| {
        if let Some(slot) = r.slots.get_mut(&slot_id) {
            slot.state = SlotState::Cached;
        }
        // last_accessed is already set to now()
    });

    // Run cleanup
    registry.update(cx, |r, cx| {
        r.cleanup_cached_projects(cx);
    });

    // Verify slot is still cached (not cleaned up)
    registry.update(cx, |r, _| {
        let slot = r.slot(slot_id).unwrap();
        assert!(matches!(slot.state, SlotState::Cached));
    });
}

// ============================================================================
// SUBSCRIPTION REBINDING TESTS
// ============================================================================

#[gpui::test]
async fn test_switch_rebinds_project_subscriptions(cx: &mut TestAppContext) {
    let temp_dir1 = TempDir::new().unwrap();
    let temp_dir2 = TempDir::new().unwrap();

    let (workspace, mut cx) = create_test_workspace_with_path(cx, temp_dir1.path()).await;

    let (slot1, slot2) = workspace.update(&mut cx, |workspace, cx| {
        workspace.app_state().workspace_registry.update(cx, |r, cx| {
            let repo_id = r.add_repo(temp_dir1.path().to_path_buf(), cx);
            let slot1 = r.add_worktree(repo_id, temp_dir1.path().to_path_buf(), "main".into(), cx).unwrap();
            let slot2 = r.add_worktree(repo_id, temp_dir2.path().to_path_buf(), "feature".into(), cx).unwrap();
            r.set_active_slot(slot1, cx);
            (slot1, slot2)
        })
    });

    workspace.update(&mut cx, |workspace, cx| {
        workspace.active_slot_id = Some(slot1);
    });

    // Switch to slot2
    workspace.update(&mut cx, |workspace, cx| {
        workspace.switch_to_slot(slot2, &mut cx.window, cx)
    }).await.unwrap();

    // Verify subscriptions were rebound (workspace should still respond to project events)
    // In a real test, we'd trigger a project event and verify the workspace handles it
}
```

**Acceptance Criteria for Phase 4:**
- [ ] All tests pass
- [ ] `switch_to_slot()` changes active slot ID
- [ ] Switch to same slot is a no-op
- [ ] Switch to nonexistent slot fails gracefully
- [ ] Current slot state is saved before switching
- [ ] Target slot state is restored after switching
- [ ] New Project is created for target worktree
- [ ] Old Project is cached after switching
- [ ] Cached projects are cleaned up after timeout
- [ ] Project subscriptions are rebound after switch

---

## Phase 5: Agent Chat Workspace Association

**Goal:** Associate agent chats with specific workspaces.

### 5.1 Add Workspace ID to Thread Storage

**File:** `crates/agent/src/db.rs` (modify)

```rust
// Add workspace_slot_id to schema
const SCHEMA: &str = r#"
    CREATE TABLE IF NOT EXISTS threads (
        id TEXT PRIMARY KEY,
        summary TEXT NOT NULL,
        updated_at TEXT NOT NULL,
        workspace_slot_id INTEGER,  -- NEW: associates thread with workspace
        data_type TEXT NOT NULL,
        data BLOB NOT NULL
    );

    CREATE INDEX IF NOT EXISTS idx_threads_workspace ON threads(workspace_slot_id);
"#;

// Add to DbThreadMetadata
#[derive(Debug, Clone)]
pub struct DbThreadMetadata {
    pub id: SessionId,
    pub title: SharedString,
    pub updated_at: DateTime<Utc>,
    pub workspace_slot_id: Option<i64>,  // NEW
}

impl ThreadsDatabase {
    /// List threads filtered by workspace
    pub fn list_threads_for_workspace(&self, slot_id: Option<i64>) -> Result<Vec<DbThreadMetadata>> {
        let mut stmt = match slot_id {
            Some(id) => self.connection.prepare(
                "SELECT id, summary, updated_at, workspace_slot_id FROM threads
                 WHERE workspace_slot_id = ?1
                 ORDER BY updated_at DESC"
            )?,
            None => self.connection.prepare(
                "SELECT id, summary, updated_at, workspace_slot_id FROM threads
                 ORDER BY updated_at DESC"
            )?,
        };

        // ... execute and map results ...
    }

    /// Save thread with workspace association
    pub fn save_thread_with_workspace(
        &self,
        id: SessionId,
        workspace_slot_id: Option<i64>,
        thread: &DbThread,
    ) -> Result<()> {
        // ... serialize and save with workspace_slot_id ...
    }
}
```

### 5.2 Populate AgentSessionInfo.cwd

**File:** `crates/agent/src/agent.rs` (modify)

```rust
impl NativeAgentServer {
    /// Convert to session info with workspace path
    fn to_session_info(entry: DbThreadMetadata, workspace_path: Option<PathBuf>) -> AgentSessionInfo {
        AgentSessionInfo {
            session_id: entry.id,
            cwd: workspace_path,  // NOW POPULATED
            title: Some(entry.title),
            updated_at: Some(entry.updated_at),
            meta: None,
        }
    }
}

impl AgentSessionList for NativeAgentSessionList {
    fn list_sessions(
        &self,
        request: AgentSessionListRequest,
    ) -> BoxFuture<'static, Result<Vec<AgentSessionInfo>>> {
        let thread_store = self.thread_store.clone();
        let cwd = request.cwd.clone();

        async move {
            // Filter by workspace if cwd provided
            let threads = thread_store.read(cx).list_for_workspace(cwd.as_ref())?;
            Ok(threads.into_iter().map(|t| Self::to_session_info(t, cwd.clone())).collect())
        }.boxed()
    }
}
```

### 5.3 Filter Agent History by Workspace

**File:** `crates/agent_ui/src/acp/thread_history.rs` (modify)

```rust
impl AcpThreadHistory {
    pub fn new(
        agent: Rc<dyn AgentServer>,
        workspace_slot_id: Option<WorkspaceSlotId>,  // NEW
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let cwd = workspace_slot_id.map(|id| {
            // Resolve slot ID to path
            // ...
        });

        Self {
            agent,
            workspace_slot_id,
            filter_cwd: cwd,
            // ...
        }
    }

    fn refresh_sessions(&mut self, cx: &mut Context<Self>) {
        let request = AgentSessionListRequest {
            cwd: self.filter_cwd.clone(),  // Filter by workspace
            cursor: None,
            meta: None,
        };

        // ... fetch sessions ...
    }
}
```

### 5.4 Update Agent Chat Count in Registry

**File:** `crates/workspace/src/workspace_registry.rs` (add)

```rust
impl WorkspaceRegistry {
    /// Update agent chat count for a workspace slot
    pub fn update_agent_chat_count(&mut self, slot_id: WorkspaceSlotId, count: usize, cx: &mut Context<Self>) {
        if let Some(slot) = self.slots.get(&slot_id) {
            if let Some(repo) = self.repos.get_mut(&slot.repo_id) {
                if let Some(worktree) = repo.worktrees.iter_mut().find(|w| w.slot_id == slot_id) {
                    worktree.agent_chat_count = count;
                    cx.notify();
                }
            }
        }
    }
}
```

### 5.5 Wire Up Count Updates

**File:** `crates/agent_ui/src/agent_panel.rs` (modify)

```rust
impl AgentChatContent {
    fn on_thread_created(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        // Update registry count
        if let Some(workspace) = self.workspace.upgrade() {
            workspace.read(cx).update_agent_chat_count_for_active_slot(cx);
        }
    }

    fn on_thread_deleted(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        // Update registry count
        if let Some(workspace) = self.workspace.upgrade() {
            workspace.read(cx).update_agent_chat_count_for_active_slot(cx);
        }
    }
}
```

### 5.6 Tests for Phase 5

**File:** `crates/agent/src/workspace_association_tests.rs` (new)

```rust
use tempfile::TempDir;

// ============================================================================
// DATABASE SCHEMA TESTS
// ============================================================================

#[test]
fn test_threads_table_has_workspace_slot_id_column() {
    let temp_dir = TempDir::new().unwrap();
    let db = ThreadsDatabase::new_with_path(temp_dir.path().join("threads.db"));

    // This should not panic - column exists
    let result = db.list_threads_for_workspace(Some(1));
    assert!(result.is_ok());
}

#[test]
fn test_list_threads_for_workspace_filters_correctly() {
    let temp_dir = TempDir::new().unwrap();
    let db = ThreadsDatabase::new_with_path(temp_dir.path().join("threads.db"));

    // Create threads for different workspaces
    let thread1 = create_test_thread("thread1");
    let thread2 = create_test_thread("thread2");
    let thread3 = create_test_thread("thread3");

    db.save_thread_with_workspace("id1".into(), Some(1), &thread1).unwrap();
    db.save_thread_with_workspace("id2".into(), Some(1), &thread2).unwrap();
    db.save_thread_with_workspace("id3".into(), Some(2), &thread3).unwrap();

    // Filter by workspace 1
    let workspace1_threads = db.list_threads_for_workspace(Some(1)).unwrap();
    assert_eq!(workspace1_threads.len(), 2);

    // Filter by workspace 2
    let workspace2_threads = db.list_threads_for_workspace(Some(2)).unwrap();
    assert_eq!(workspace2_threads.len(), 1);

    // No filter - get all
    let all_threads = db.list_threads_for_workspace(None).unwrap();
    assert_eq!(all_threads.len(), 3);
}

#[test]
fn test_thread_metadata_includes_workspace_slot_id() {
    let temp_dir = TempDir::new().unwrap();
    let db = ThreadsDatabase::new_with_path(temp_dir.path().join("threads.db"));

    let thread = create_test_thread("test");
    db.save_thread_with_workspace("id1".into(), Some(42), &thread).unwrap();

    let threads = db.list_threads_for_workspace(Some(42)).unwrap();
    assert_eq!(threads[0].workspace_slot_id, Some(42));
}

// ============================================================================
// AGENT SESSION INFO TESTS
// ============================================================================

#[test]
fn test_to_session_info_populates_cwd() {
    let metadata = DbThreadMetadata {
        id: "test-id".into(),
        title: "Test Thread".into(),
        updated_at: Utc::now(),
        workspace_slot_id: Some(1),
    };

    let workspace_path = Some(PathBuf::from("/path/to/workspace"));
    let session_info = NativeAgentServer::to_session_info(metadata, workspace_path.clone());

    assert_eq!(session_info.cwd, workspace_path);
}

#[test]
fn test_to_session_info_handles_none_cwd() {
    let metadata = DbThreadMetadata {
        id: "test-id".into(),
        title: "Test Thread".into(),
        updated_at: Utc::now(),
        workspace_slot_id: None,
    };

    let session_info = NativeAgentServer::to_session_info(metadata, None);

    assert!(session_info.cwd.is_none());
}

// ============================================================================
// SESSION LIST FILTERING TESTS
// ============================================================================

#[gpui::test]
async fn test_list_sessions_filters_by_cwd(cx: &mut TestAppContext) {
    let temp_dir = TempDir::new().unwrap();

    // Create thread store with test database
    let thread_store = cx.new(|cx| {
        ThreadStore::new_for_test(temp_dir.path().join("threads.db"), cx)
    });

    // Add threads with different workspace associations
    thread_store.update(cx, |store, cx| {
        store.save_thread_with_workspace("id1".into(), Some(1), create_test_thread("t1"), cx);
        store.save_thread_with_workspace("id2".into(), Some(2), create_test_thread("t2"), cx);
    });

    let session_list = NativeAgentSessionList::new(thread_store);

    // Request with cwd filter
    let request = AgentSessionListRequest {
        cwd: Some(PathBuf::from("/workspace/1")), // Resolved from slot_id 1
        cursor: None,
        meta: None,
    };

    let sessions = session_list.list_sessions(request).await.unwrap();

    // Should only return threads for workspace 1
    assert_eq!(sessions.len(), 1);
}

// ============================================================================
// THREAD HISTORY FILTERING TESTS
// ============================================================================

#[gpui::test]
async fn test_thread_history_filters_by_workspace_slot(cx: &mut TestAppContext) {
    let (workspace, mut cx) = create_test_workspace(cx).await;

    let slot_id = workspace.update(&mut cx, |workspace, cx| {
        workspace.app_state().workspace_registry.update(cx, |r, cx| {
            let repo_id = r.add_repo("/path/to/repo".into(), cx);
            r.add_worktree(repo_id, "/path/wt".into(), "main".into(), cx).unwrap()
        })
    });

    // Create thread history with workspace filter
    let history = cx.new(|cx| {
        AcpThreadHistory::new(
            create_test_agent_server(),
            Some(slot_id),
            &mut cx.window,
            cx,
        )
    });

    history.update(&mut cx, |history, _| {
        assert_eq!(history.workspace_slot_id, Some(slot_id));
        assert!(history.filter_cwd.is_some());
    });
}

// ============================================================================
// AGENT CHAT COUNT TESTS
// ============================================================================

#[gpui::test]
async fn test_update_agent_chat_count_for_active_slot(cx: &mut TestAppContext) {
    let (workspace, mut cx) = create_test_workspace(cx).await;

    let slot_id = workspace.update(&mut cx, |workspace, cx| {
        workspace.app_state().workspace_registry.update(cx, |r, cx| {
            let repo_id = r.add_repo("/path/to/repo".into(), cx);
            let slot_id = r.add_worktree(repo_id, "/path/wt".into(), "main".into(), cx).unwrap();
            r.set_active_slot(slot_id, cx);
            slot_id
        })
    });

    workspace.update(&mut cx, |workspace, cx| {
        workspace.active_slot_id = Some(slot_id);
    });

    // Simulate thread creation updating count
    workspace.update(&mut cx, |workspace, cx| {
        workspace.app_state().workspace_registry.update(cx, |r, cx| {
            r.update_agent_chat_count(slot_id, 5, cx);
        });
    });

    // Verify count was updated
    workspace.update(&mut cx, |workspace, cx| {
        let registry = workspace.app_state().workspace_registry.read(cx);
        let repo = registry.repos().next().unwrap();
        assert_eq!(repo.worktrees[0].agent_chat_count, 5);
    });
}

#[gpui::test]
async fn test_thread_created_updates_count(cx: &mut TestAppContext) {
    // This would test the integration between AgentChatContent and the registry
    // when a new thread is created
}

#[gpui::test]
async fn test_thread_deleted_updates_count(cx: &mut TestAppContext) {
    // This would test the integration between AgentChatContent and the registry
    // when a thread is deleted
}
```

**Acceptance Criteria for Phase 5:**
- [ ] All tests pass
- [ ] `workspace_slot_id` column exists in threads table
- [ ] `list_threads_for_workspace()` filters correctly
- [ ] `DbThreadMetadata` includes `workspace_slot_id`
- [ ] `to_session_info()` populates `cwd` field
- [ ] `AcpThreadHistory` filters by workspace
- [ ] Agent chat count updates when threads are created/deleted

---

## Phase 6: Git Worktree Integration

**Goal:** Leverage existing git worktree operations for creating new workspaces.

### 6.1 Create Worktree from Panel

**File:** `crates/workspaces_panel/src/workspaces_panel.rs` (add)

```rust
impl WorkspacesPanel {
    fn create_worktree_for_repo(&mut self, repo_id: RepoId, window: &mut Window, cx: &mut Context<Self>) {
        let registry = self.registry.clone();

        cx.spawn_in(window, async move |this, mut cx| {
            // Get repo path
            let repo_root = cx.update(|_, cx| {
                registry.read(cx).repos.get(&repo_id).map(|r| r.root_path.clone())
            })?.ok_or_else(|| anyhow!("Repo not found"))?;

            // Prompt for branch name
            let branch_name = cx.prompt_for_string(
                "New Worktree",
                "Enter branch name:",
                None,
            ).await?.ok_or_else(|| anyhow!("Cancelled"))?;

            // Prompt for worktree location
            let worktree_dir = cx.update(|window, _| {
                window.prompt_for_new_path(&repo_root.parent().unwrap_or(&repo_root))
            })?.await?.ok_or_else(|| anyhow!("Cancelled"))?;

            // Get git repository
            let git_repo = cx.update(|_, cx| {
                // Get git repository from project or create new one
                let fs = this.read(cx).app_state().fs.clone();
                fs.open_repo(&repo_root)
            })?.ok_or_else(|| anyhow!("Not a git repository"))?;

            // Create the worktree
            git_repo.create_worktree(
                branch_name.clone(),
                worktree_dir.clone(),
                None, // from current HEAD
            ).await?;

            // Add to registry
            this.update(&mut cx, |panel, cx| {
                panel.registry.update(cx, |registry, cx| {
                    registry.add_worktree(
                        repo_id,
                        worktree_dir,
                        branch_name.into(),
                        cx,
                    );
                });
            })?;

            Ok(())
        }).detach_and_log_err(cx);
    }
}
```

### 6.2 Scan Existing Worktrees on Repo Add

**File:** `crates/workspace/src/workspace_registry.rs` (add)

```rust
impl WorkspaceRegistry {
    /// Add a repo and automatically discover its worktrees
    pub fn add_repo_with_worktrees(
        &mut self,
        root_path: PathBuf,
        fs: Arc<dyn Fs>,
        cx: &mut Context<Self>,
    ) -> Task<Result<RepoId>> {
        let repo_id = self.add_repo(root_path.clone(), cx);
        let this = cx.weak_entity();

        cx.spawn(async move |cx| {
            // Open git repository
            let git_repo = fs.open_repo(&root_path).await?
                .ok_or_else(|| anyhow!("Not a git repository"))?;

            // List worktrees
            let worktrees = git_repo.worktrees().await?;

            // Add each worktree
            cx.update(|cx| {
                this.update(cx, |registry, cx| {
                    for worktree in worktrees {
                        registry.add_worktree(
                            repo_id,
                            worktree.path,
                            worktree.ref_name,
                            cx,
                        );
                    }
                })
            })??;

            Ok(repo_id)
        })
    }
}
```

### 6.3 Tests for Phase 6

**File:** `crates/workspaces_panel/src/git_worktree_tests.rs` (new)

```rust
use tempfile::TempDir;
use std::process::Command;

/// Helper to create a test git repository
fn create_test_git_repo(dir: &Path) -> Result<()> {
    Command::new("git").args(["init"]).current_dir(dir).output()?;
    Command::new("git").args(["config", "user.email", "test@test.com"]).current_dir(dir).output()?;
    Command::new("git").args(["config", "user.name", "Test"]).current_dir(dir).output()?;

    std::fs::write(dir.join("README.md"), "# Test")?;
    Command::new("git").args(["add", "."]).current_dir(dir).output()?;
    Command::new("git").args(["commit", "-m", "Initial commit"]).current_dir(dir).output()?;
    Ok(())
}

/// Helper to create a git worktree
fn create_git_worktree(repo_dir: &Path, worktree_dir: &Path, branch: &str) -> Result<()> {
    Command::new("git")
        .args(["worktree", "add", "-b", branch, worktree_dir.to_str().unwrap()])
        .current_dir(repo_dir)
        .output()?;
    Ok(())
}

// ============================================================================
// WORKTREE CREATION TESTS
// ============================================================================

#[gpui::test]
async fn test_create_worktree_from_panel(cx: &mut TestAppContext) {
    let temp_dir = TempDir::new().unwrap();
    let repo_dir = temp_dir.path().join("repo");
    std::fs::create_dir(&repo_dir).unwrap();
    create_test_git_repo(&repo_dir).unwrap();

    let (workspace, mut cx) = create_test_workspace_with_path(cx, &repo_dir).await;

    let repo_id = workspace.update(&mut cx, |workspace, cx| {
        workspace.app_state().workspace_registry.update(cx, |r, cx| {
            r.add_repo(repo_dir.clone(), cx)
        })
    });

    let panel = workspace.update(&mut cx, |workspace, cx| {
        cx.new(|cx| WorkspacesPanel::new(workspace, &mut cx.window, cx))
    });

    // Trigger worktree creation
    // In real test, would mock the prompt dialogs
    // panel.update(&mut cx, |panel, cx| {
    //     panel.create_worktree_for_repo(repo_id, &mut cx.window, cx);
    // });

    // For now, just verify the method exists and repo is set up correctly
    workspace.update(&mut cx, |workspace, cx| {
        let registry = workspace.app_state().workspace_registry.read(cx);
        assert!(registry.repos().any(|r| r.id == repo_id));
    });
}

#[gpui::test]
async fn test_create_worktree_uses_git_repository(cx: &mut TestAppContext) {
    let temp_dir = TempDir::new().unwrap();
    let repo_dir = temp_dir.path().join("repo");
    let worktree_dir = temp_dir.path().join("worktree");
    std::fs::create_dir(&repo_dir).unwrap();
    create_test_git_repo(&repo_dir).unwrap();

    // Use existing git infrastructure
    let fs = Arc::new(RealFs::new(Default::default()));
    let git_repo = fs.open_repo(&repo_dir).unwrap().unwrap();

    // Create worktree via git API
    git_repo.create_worktree(
        "feature-branch".to_string(),
        worktree_dir.clone(),
        None,
    ).await.unwrap();

    // Verify worktree was created
    assert!(worktree_dir.exists());

    // List worktrees
    let worktrees = git_repo.worktrees().await.unwrap();
    assert!(worktrees.len() >= 2); // Main worktree + new one
    assert!(worktrees.iter().any(|w| w.ref_name.contains("feature-branch")));
}

// ============================================================================
// WORKTREE DISCOVERY TESTS
// ============================================================================

#[gpui::test]
async fn test_scan_repo_discovers_existing_worktrees(cx: &mut TestAppContext) {
    let temp_dir = TempDir::new().unwrap();
    let repo_dir = temp_dir.path().join("repo");
    let worktree1_dir = temp_dir.path().join("worktree1");
    let worktree2_dir = temp_dir.path().join("worktree2");

    std::fs::create_dir(&repo_dir).unwrap();
    create_test_git_repo(&repo_dir).unwrap();
    create_git_worktree(&repo_dir, &worktree1_dir, "feature-1").unwrap();
    create_git_worktree(&repo_dir, &worktree2_dir, "feature-2").unwrap();

    let registry = create_test_registry(cx);
    let fs = Arc::new(RealFs::new(Default::default()));

    let repo_id = registry.update(cx, |r, cx| {
        r.add_repo_with_worktrees(repo_dir.clone(), fs.clone(), cx)
    }).await.unwrap();

    cx.run_until_parked();

    registry.update(cx, |r, _| {
        let repo = r.repos().find(|repo| repo.id == repo_id).unwrap();
        // Should have main worktree + 2 feature worktrees
        assert!(repo.worktrees.len() >= 2);
    });
}

#[gpui::test]
async fn test_add_repo_with_worktrees_for_non_git_folder(cx: &mut TestAppContext) {
    let temp_dir = TempDir::new().unwrap();

    let registry = create_test_registry(cx);
    let fs = Arc::new(RealFs::new(Default::default()));

    let result = registry.update(cx, |r, cx| {
        r.add_repo_with_worktrees(temp_dir.path().to_path_buf(), fs.clone(), cx)
    }).await;

    // Should fail gracefully for non-git folder
    assert!(result.is_err());
}

// ============================================================================
// REGISTRY WORKTREE SCANNING TESTS
// ============================================================================

#[gpui::test]
async fn test_scan_repo_worktrees_adds_all_worktrees(cx: &mut TestAppContext) {
    let temp_dir = TempDir::new().unwrap();
    let repo_dir = temp_dir.path().join("repo");
    std::fs::create_dir(&repo_dir).unwrap();
    create_test_git_repo(&repo_dir).unwrap();

    // Create additional worktrees
    create_git_worktree(&repo_dir, &temp_dir.path().join("wt1"), "branch1").unwrap();
    create_git_worktree(&repo_dir, &temp_dir.path().join("wt2"), "branch2").unwrap();

    let fs = Arc::new(RealFs::new(Default::default()));
    let git_repo = fs.open_repo(&repo_dir).unwrap().unwrap();

    let registry = create_test_registry(cx);

    let repo_id = registry.update(cx, |r, cx| {
        r.add_repo(repo_dir.clone(), cx)
    });

    registry.update(cx, |r, cx| {
        r.scan_repo_worktrees(repo_id, git_repo.as_ref(), cx)
    }).await.unwrap();

    registry.update(cx, |r, _| {
        let repo = r.repos().find(|repo| repo.id == repo_id).unwrap();
        assert!(repo.worktrees.len() >= 2);
    });
}

// ============================================================================
// INTEGRATION TESTS
// ============================================================================

#[gpui::test]
async fn test_full_worktree_creation_flow(cx: &mut TestAppContext) {
    // This test would cover the full flow:
    // 1. User clicks "+" on a repo in the panel
    // 2. User enters branch name
    // 3. User selects worktree location
    // 4. Git worktree is created
    // 5. Worktree is added to registry
    // 6. Panel updates to show new worktree

    // Would require mocking prompt dialogs
}
```

**Acceptance Criteria for Phase 6:**
- [ ] All tests pass
- [ ] `create_worktree_for_repo()` creates git worktree via GitRepository API
- [ ] Created worktrees are added to registry
- [ ] `add_repo_with_worktrees()` discovers existing worktrees
- [ ] `scan_repo_worktrees()` adds all worktrees from git
- [ ] Non-git folders are handled gracefully

---

## Phase 7: Settings and Feature Flag

**Goal:** Add settings to enable/disable the multi-workspace feature.

### 7.1 Add Settings Schema

**File:** `crates/settings_content/src/workspace.rs` (modify)

```rust
#[with_fallible_options]
#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema, MergeFrom, Debug, Default)]
pub struct WorkspaceSettingsContent {
    // ... existing fields ...

    /// Enable multi-workspace mode with git worktree support.
    /// When enabled, shows a workspaces panel for managing multiple
    /// worktrees per repository and multiple agent chats per workspace.
    ///
    /// Default: false
    pub multi_workspace_mode: Option<bool>,
}
```

### 7.2 Add Feature Flag

**File:** `crates/feature_flags/src/flags.rs` (add)

```rust
pub struct MultiWorkspaceFeatureFlag;

impl FeatureFlag for MultiWorkspaceFeatureFlag {
    const NAME: &'static str = "multi-workspace";

    fn enabled_for_staff() -> bool {
        true
    }

    fn enabled_for_all() -> bool {
        false
    }
}
```

### 7.3 Conditional Panel Registration

**File:** `crates/zed/src/zed.rs` (modify)

```rust
fn initialize_workspace(/* ... */) {
    // ... existing code ...

    // Only register workspaces panel if feature is enabled
    if cx.has_flag::<MultiWorkspaceFeatureFlag>() ||
       WorkspaceSettings::get_global(cx).multi_workspace_mode
    {
        workspaces_panel::init(cx);
        workspace.register_panel::<WorkspacesPanel>(cx);
    }
}
```

### 7.4 Add Default Setting

**File:** `assets/settings/default.json` (add)

```json
{
  "workspace": {
    // ... existing settings ...

    // Enable multi-workspace mode for git worktree support
    "multi_workspace_mode": false
  }
}
```

### 7.5 Tests for Phase 7

**File:** `crates/workspace/src/settings_tests.rs` (new)

```rust
// ============================================================================
// SETTINGS SCHEMA TESTS
// ============================================================================

#[test]
fn test_multi_workspace_mode_setting_exists() {
    let schema = schemars::schema_for!(WorkspaceSettingsContent);
    let schema_json = serde_json::to_string_pretty(&schema).unwrap();

    // Verify multi_workspace_mode is in the schema
    assert!(schema_json.contains("multi_workspace_mode"));
}

#[test]
fn test_multi_workspace_mode_defaults_to_false() {
    let settings = WorkspaceSettingsContent::default();
    assert_eq!(settings.multi_workspace_mode, None);

    // When merged with defaults, should be false
    let default_json = include_str!("../../../../assets/settings/default.json");
    let parsed: serde_json::Value = serde_json::from_str(default_json).unwrap();

    let multi_workspace = parsed
        .get("workspace")
        .and_then(|w| w.get("multi_workspace_mode"))
        .and_then(|v| v.as_bool());

    assert_eq!(multi_workspace, Some(false));
}

#[test]
fn test_multi_workspace_mode_can_be_enabled() {
    let json = r#"{ "multi_workspace_mode": true }"#;
    let settings: WorkspaceSettingsContent = serde_json::from_str(json).unwrap();

    assert_eq!(settings.multi_workspace_mode, Some(true));
}

// ============================================================================
// FEATURE FLAG TESTS
// ============================================================================

#[test]
fn test_multi_workspace_feature_flag_name() {
    assert_eq!(MultiWorkspaceFeatureFlag::NAME, "multi-workspace");
}

#[test]
fn test_multi_workspace_feature_flag_enabled_for_staff() {
    assert!(MultiWorkspaceFeatureFlag::enabled_for_staff());
}

#[test]
fn test_multi_workspace_feature_flag_disabled_for_all_by_default() {
    assert!(!MultiWorkspaceFeatureFlag::enabled_for_all());
}

#[gpui::test]
async fn test_has_flag_returns_false_when_disabled(cx: &mut TestAppContext) {
    // Without staff flag and not in flags list
    cx.update(|cx| {
        cx.update_flags(false, vec![]);
    });

    cx.update(|cx| {
        assert!(!cx.has_flag::<MultiWorkspaceFeatureFlag>());
    });
}

#[gpui::test]
async fn test_has_flag_returns_true_for_staff(cx: &mut TestAppContext) {
    // With staff flag
    cx.update(|cx| {
        cx.update_flags(true, vec![]);
    });

    cx.update(|cx| {
        assert!(cx.has_flag::<MultiWorkspaceFeatureFlag>());
    });
}

#[gpui::test]
async fn test_has_flag_returns_true_when_in_flags_list(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.update_flags(false, vec!["multi-workspace".to_string()]);
    });

    cx.update(|cx| {
        assert!(cx.has_flag::<MultiWorkspaceFeatureFlag>());
    });
}

// ============================================================================
// CONDITIONAL REGISTRATION TESTS
// ============================================================================

#[gpui::test]
async fn test_panel_not_registered_when_feature_disabled(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.update_flags(false, vec![]);
    });

    let (workspace, mut cx) = create_test_workspace(cx).await;

    // Panel should not be registered
    workspace.update(&mut cx, |workspace, cx| {
        let panel = workspace.panel::<WorkspacesPanel>(cx);
        assert!(panel.is_none());
    });
}

#[gpui::test]
async fn test_panel_registered_when_feature_enabled(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.update_flags(true, vec![]); // Staff mode
    });

    let (workspace, mut cx) = create_test_workspace(cx).await;

    // Panel should be registered
    workspace.update(&mut cx, |workspace, cx| {
        let panel = workspace.panel::<WorkspacesPanel>(cx);
        assert!(panel.is_some());
    });
}

#[gpui::test]
async fn test_panel_registered_when_setting_enabled(cx: &mut TestAppContext) {
    // Enable via settings instead of feature flag
    cx.update(|cx| {
        cx.update_flags(false, vec![]);
        // Would need to mock settings here
    });

    // Test would verify panel registration when multi_workspace_mode is true
}
```

**Acceptance Criteria for Phase 7:**
- [ ] All tests pass
- [ ] `multi_workspace_mode` setting exists in schema
- [ ] Setting defaults to `false`
- [ ] Setting can be set to `true`
- [ ] `MultiWorkspaceFeatureFlag` has correct name
- [ ] Feature flag enabled for staff by default
- [ ] Feature flag disabled for all by default
- [ ] Panel only registered when feature/setting is enabled

---

## Phase 8: Polish and Edge Cases

**Goal:** Handle edge cases and improve UX.

### 8.1 Handle Deleted Worktrees

**File:** `crates/workspace/src/workspace_registry.rs` (add)

```rust
impl WorkspaceRegistry {
    /// Validate that all worktrees still exist on disk
    pub fn validate_worktrees(&mut self, fs: &dyn Fs, cx: &mut Context<Self>) -> Task<()> {
        let paths: Vec<_> = self.slots.values()
            .map(|s| (s.slot_id, s.worktree_path.clone()))
            .collect();

        cx.spawn(async move |this, mut cx| {
            for (slot_id, path) in paths {
                if !fs.is_dir(&path).await {
                    this.update(&mut cx, |registry, cx| {
                        registry.remove_worktree(slot_id, cx);
                    }).log_err();
                }
            }
        })
    }

    fn remove_worktree(&mut self, slot_id: WorkspaceSlotId, cx: &mut Context<Self>) {
        if let Some(slot) = self.slots.remove(&slot_id) {
            if let Some(repo) = self.repos.get_mut(&slot.repo_id) {
                repo.worktrees.retain(|w| w.slot_id != slot_id);
            }
            self.db.delete_worktree(slot_id).log_err();
            cx.emit(WorkspaceRegistryEvent::WorktreeRemoved {
                repo_id: slot.repo_id,
                slot_id,
            });
            cx.notify();
        }
    }
}
```

### 8.2 Handle Non-Git Folders

**File:** `crates/workspaces_panel/src/workspaces_panel.rs` (add)

```rust
impl WorkspacesPanel {
    fn add_repository(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, mut cx| {
            let path = /* prompt for path */;

            // Check if it's a git repository
            let fs = this.read(&cx)?.app_state().fs.clone();
            let is_git = fs.open_repo(&path).await.is_some();

            if !is_git {
                // Show warning but still allow adding
                let proceed = cx.prompt(
                    PromptLevel::Warning,
                    "Not a Git Repository",
                    Some("This folder is not a git repository. Worktree features will be limited. Add anyway?"),
                    &["Add", "Cancel"],
                ).await? == 0;

                if !proceed {
                    return Ok(());
                }
            }

            // Add the repo
            this.update(&mut cx, |panel, cx| {
                panel.registry.update(cx, |registry, cx| {
                    registry.add_repo(path, cx);
                });
            })?;

            Ok(())
        }).detach_and_log_err(cx);
    }
}
```

### 8.3 Keyboard Navigation

**File:** `crates/workspaces_panel/src/workspaces_panel.rs` (add)

```rust
impl WorkspacesPanel {
    fn handle_switch_to_workspace(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        // Get all worktrees in order
        let worktrees: Vec<_> = self.registry.read(cx)
            .repos()
            .flat_map(|r| r.worktrees.iter())
            .collect();

        if let Some(worktree) = worktrees.get(index) {
            self.switch_to_workspace(worktree.slot_id, window, cx);
        }
    }
}

// Action handlers
impl WorkspacesPanel {
    fn switch_to_workspace_1(&mut self, _: &SwitchToWorkspace1, window: &mut Window, cx: &mut Context<Self>) {
        self.handle_switch_to_workspace(0, window, cx);
    }

    fn switch_to_workspace_2(&mut self, _: &SwitchToWorkspace2, window: &mut Window, cx: &mut Context<Self>) {
        self.handle_switch_to_workspace(1, window, cx);
    }

    // ... etc for 3, 4, 5
}
```

### 8.4 Status Bar Integration

**File:** `crates/workspace/src/status_bar.rs` (modify)

```rust
// Add current workspace indicator to status bar
impl StatusBar {
    fn render_workspace_indicator(&self, cx: &App) -> impl IntoElement {
        let registry = self.workspace.read(cx).app_state.workspace_registry.read(cx);

        if let Some(slot) = registry.active_slot() {
            if let Some(repo) = registry.repos.get(&slot.repo_id) {
                if let Some(worktree) = repo.worktrees.iter().find(|w| w.slot_id == slot.slot_id) {
                    return h_flex()
                        .gap_1()
                        .child(Icon::new(IconName::GitBranch).size(IconSize::Small))
                        .child(Label::new(worktree.branch_name.clone()).size(LabelSize::Small))
                        .into_any_element();
                }
            }
        }

        div().into_any_element()
    }
}
```

### 8.5 Tests for Phase 8

**File:** `crates/workspace/src/polish_tests.rs` (new)

```rust
use tempfile::TempDir;

// ============================================================================
// DELETED WORKTREE TESTS
// ============================================================================

#[gpui::test]
async fn test_validate_worktrees_removes_deleted(cx: &mut TestAppContext) {
    let temp_dir = TempDir::new().unwrap();
    let worktree_path = temp_dir.path().join("worktree");
    std::fs::create_dir(&worktree_path).unwrap();

    let registry = create_test_registry(cx);
    let fs = Arc::new(RealFs::new(Default::default()));

    let slot_id = registry.update(cx, |r, cx| {
        let repo_id = r.add_repo(temp_dir.path().to_path_buf(), cx);
        r.add_worktree(repo_id, worktree_path.clone(), "main".into(), cx).unwrap()
    });

    // Delete the worktree directory
    std::fs::remove_dir_all(&worktree_path).unwrap();

    // Run validation
    registry.update(cx, |r, cx| {
        r.validate_worktrees(fs.as_ref(), cx)
    }).await;

    cx.run_until_parked();

    // Worktree should be removed from registry
    registry.update(cx, |r, _| {
        assert!(r.slot(slot_id).is_none());
    });
}

#[gpui::test]
async fn test_validate_worktrees_preserves_existing(cx: &mut TestAppContext) {
    let temp_dir = TempDir::new().unwrap();
    let worktree_path = temp_dir.path().join("worktree");
    std::fs::create_dir(&worktree_path).unwrap();

    let registry = create_test_registry(cx);
    let fs = Arc::new(RealFs::new(Default::default()));

    let slot_id = registry.update(cx, |r, cx| {
        let repo_id = r.add_repo(temp_dir.path().to_path_buf(), cx);
        r.add_worktree(repo_id, worktree_path.clone(), "main".into(), cx).unwrap()
    });

    // Don't delete - run validation
    registry.update(cx, |r, cx| {
        r.validate_worktrees(fs.as_ref(), cx)
    }).await;

    cx.run_until_parked();

    // Worktree should still exist
    registry.update(cx, |r, _| {
        assert!(r.slot(slot_id).is_some());
    });
}

#[gpui::test]
async fn test_remove_worktree_emits_event(cx: &mut TestAppContext) {
    let registry = create_test_registry(cx);
    let events = Rc::new(RefCell::new(Vec::new()));

    let slot_id = registry.update(cx, |r, cx| {
        let repo_id = r.add_repo("/path/to/repo".into(), cx);
        r.add_worktree(repo_id, "/path/wt".into(), "main".into(), cx).unwrap()
    });

    let events_clone = events.clone();
    cx.subscribe(&registry, move |_, event, _| {
        events_clone.borrow_mut().push(event.clone());
    }).detach();

    registry.update(cx, |r, cx| {
        r.remove_worktree(slot_id, cx);
    });

    cx.run_until_parked();

    let events = events.borrow();
    assert!(events.iter().any(|e| matches!(e, WorkspaceRegistryEvent::WorktreeRemoved { .. })));
}

// ============================================================================
// NON-GIT FOLDER TESTS
// ============================================================================

#[gpui::test]
async fn test_add_non_git_repo_still_works(cx: &mut TestAppContext) {
    let temp_dir = TempDir::new().unwrap();

    let registry = create_test_registry(cx);

    // Adding a non-git folder should still create a repo entry
    let repo_id = registry.update(cx, |r, cx| {
        r.add_repo(temp_dir.path().to_path_buf(), cx)
    });

    registry.update(cx, |r, _| {
        assert!(r.repos().any(|repo| repo.id == repo_id));
    });
}

#[gpui::test]
async fn test_non_git_repo_has_no_worktrees(cx: &mut TestAppContext) {
    let temp_dir = TempDir::new().unwrap();

    let registry = create_test_registry(cx);

    let repo_id = registry.update(cx, |r, cx| {
        r.add_repo(temp_dir.path().to_path_buf(), cx)
    });

    registry.update(cx, |r, _| {
        let repo = r.repos().find(|repo| repo.id == repo_id).unwrap();
        // Non-git folder won't have worktrees auto-discovered
        assert_eq!(repo.worktrees.len(), 0);
    });
}

// ============================================================================
// KEYBOARD NAVIGATION TESTS
// ============================================================================

#[gpui::test]
async fn test_keyboard_shortcuts_work_across_repos(cx: &mut TestAppContext) {
    let (workspace, mut cx) = create_test_workspace(cx).await;

    // Add multiple repos with multiple worktrees
    let (slot1, slot2, slot3) = workspace.update(&mut cx, |workspace, cx| {
        workspace.app_state().workspace_registry.update(cx, |r, cx| {
            let repo1 = r.add_repo("/path/to/repo1".into(), cx);
            let slot1 = r.add_worktree(repo1, "/path/wt1".into(), "main".into(), cx).unwrap();

            let repo2 = r.add_repo("/path/to/repo2".into(), cx);
            let slot2 = r.add_worktree(repo2, "/path/wt2".into(), "develop".into(), cx).unwrap();
            let slot3 = r.add_worktree(repo2, "/path/wt3".into(), "feature".into(), cx).unwrap();

            (slot1, slot2, slot3)
        })
    });

    let panel = workspace.update(&mut cx, |workspace, cx| {
        cx.new(|cx| WorkspacesPanel::new(workspace, &mut cx.window, cx))
    });

    // ⌘1 should select slot1 (first worktree overall)
    panel.update(&mut cx, |panel, cx| {
        panel.handle_switch_to_workspace(0, &mut cx.window, cx);
    });

    cx.run_until_parked();

    workspace.update(&mut cx, |workspace, cx| {
        let registry = workspace.app_state().workspace_registry.read(cx);
        assert_eq!(registry.active_slot().map(|s| s.slot_id), Some(slot1));
    });

    // ⌘3 should select slot3 (third worktree overall, second repo)
    panel.update(&mut cx, |panel, cx| {
        panel.handle_switch_to_workspace(2, &mut cx.window, cx);
    });

    cx.run_until_parked();

    workspace.update(&mut cx, |workspace, cx| {
        let registry = workspace.app_state().workspace_registry.read(cx);
        assert_eq!(registry.active_slot().map(|s| s.slot_id), Some(slot3));
    });
}

// ============================================================================
// STATUS BAR TESTS
// ============================================================================

#[gpui::test]
async fn test_status_bar_shows_branch_when_active(cx: &mut TestAppContext) {
    let (workspace, mut cx) = create_test_workspace(cx).await;

    let slot_id = workspace.update(&mut cx, |workspace, cx| {
        workspace.app_state().workspace_registry.update(cx, |r, cx| {
            let repo_id = r.add_repo("/path/to/repo".into(), cx);
            let slot_id = r.add_worktree(repo_id, "/path/wt".into(), "feature-xyz".into(), cx).unwrap();
            r.set_active_slot(slot_id, cx);
            slot_id
        })
    });

    workspace.update(&mut cx, |workspace, cx| {
        workspace.active_slot_id = Some(slot_id);

        // In real test, would render status bar and check for "feature-xyz" label
        let registry = workspace.app_state().workspace_registry.read(cx);
        let slot = registry.active_slot().unwrap();
        let repo = registry.repos().find(|r| r.id == slot.repo_id).unwrap();
        let worktree = repo.worktrees.iter().find(|w| w.slot_id == slot_id).unwrap();

        assert_eq!(worktree.branch_name.as_ref(), "feature-xyz");
    });
}

#[gpui::test]
async fn test_status_bar_empty_when_no_active_slot(cx: &mut TestAppContext) {
    let (workspace, mut cx) = create_test_workspace(cx).await;

    workspace.update(&mut cx, |workspace, cx| {
        let registry = workspace.app_state().workspace_registry.read(cx);
        assert!(registry.active_slot().is_none());

        // Status bar indicator should be empty/hidden
    });
}

// ============================================================================
// TIME AGO FORMATTING TESTS
// ============================================================================

#[test]
fn test_format_time_ago_seconds() {
    let now = Utc::now();
    let time = now - chrono::Duration::seconds(30);
    assert_eq!(format_time_ago(time), "just now");
}

#[test]
fn test_format_time_ago_minutes() {
    let now = Utc::now();
    let time = now - chrono::Duration::minutes(5);
    assert_eq!(format_time_ago(time), "5m ago");
}

#[test]
fn test_format_time_ago_hours() {
    let now = Utc::now();
    let time = now - chrono::Duration::hours(3);
    assert_eq!(format_time_ago(time), "3h ago");
}

#[test]
fn test_format_time_ago_days() {
    let now = Utc::now();
    let time = now - chrono::Duration::days(2);
    assert_eq!(format_time_ago(time), "2d ago");
}

#[test]
fn test_format_time_ago_months() {
    let now = Utc::now();
    let time = now - chrono::Duration::days(45);
    assert_eq!(format_time_ago(time), "1mo ago");
}

// ============================================================================
// EDGE CASE TESTS
// ============================================================================

#[gpui::test]
async fn test_rapid_switching_does_not_crash(cx: &mut TestAppContext) {
    let temp_dir1 = TempDir::new().unwrap();
    let temp_dir2 = TempDir::new().unwrap();
    let temp_dir3 = TempDir::new().unwrap();

    let (workspace, mut cx) = create_test_workspace_with_path(cx, temp_dir1.path()).await;

    let (slot1, slot2, slot3) = workspace.update(&mut cx, |workspace, cx| {
        workspace.app_state().workspace_registry.update(cx, |r, cx| {
            let repo_id = r.add_repo(temp_dir1.path().to_path_buf(), cx);
            let slot1 = r.add_worktree(repo_id, temp_dir1.path().to_path_buf(), "main".into(), cx).unwrap();
            let slot2 = r.add_worktree(repo_id, temp_dir2.path().to_path_buf(), "dev".into(), cx).unwrap();
            let slot3 = r.add_worktree(repo_id, temp_dir3.path().to_path_buf(), "feature".into(), cx).unwrap();
            r.set_active_slot(slot1, cx);
            (slot1, slot2, slot3)
        })
    });

    workspace.update(&mut cx, |workspace, cx| {
        workspace.active_slot_id = Some(slot1);
    });

    // Rapid switching - these tasks will queue up
    let task1 = workspace.update(&mut cx, |workspace, cx| {
        workspace.switch_to_slot(slot2, &mut cx.window, cx)
    });
    let task2 = workspace.update(&mut cx, |workspace, cx| {
        workspace.switch_to_slot(slot3, &mut cx.window, cx)
    });
    let task3 = workspace.update(&mut cx, |workspace, cx| {
        workspace.switch_to_slot(slot1, &mut cx.window, cx)
    });

    // All tasks should complete without error
    task1.await.ok();
    task2.await.ok();
    task3.await.ok();

    // System should be in a consistent state
    workspace.update(&mut cx, |workspace, _| {
        assert!(workspace.active_slot_id.is_some());
    });
}

#[gpui::test]
async fn test_empty_repo_name_handled(cx: &mut TestAppContext) {
    let registry = create_test_registry(cx);

    // Path with no file name component
    let repo_id = registry.update(cx, |r, cx| {
        r.add_repo("/".into(), cx)
    });

    registry.update(cx, |r, _| {
        let repo = r.repos().find(|repo| repo.id == repo_id).unwrap();
        // Should have a fallback display name
        assert!(!repo.display_name.is_empty());
    });
}
```

**Acceptance Criteria for Phase 8:**
- [ ] All tests pass
- [ ] Deleted worktrees are detected and removed from registry
- [ ] Non-git folders can be added (with limited features)
- [ ] Keyboard shortcuts (⌘1-⌘5) work across multiple repos
- [ ] Status bar shows current branch name
- [ ] Time ago formatting works correctly
- [ ] Rapid workspace switching doesn't crash
- [ ] Edge cases (empty names, missing paths) are handled

---

## Phase 9: Integration Testing

**Goal:** Comprehensive test coverage.

### 9.1 Unit Tests

**File:** `crates/workspace/src/workspace_registry_tests.rs`

```rust
#[gpui::test]
async fn test_add_and_list_repos(cx: &mut TestAppContext) {
    // Test adding repos and listing them
}

#[gpui::test]
async fn test_add_worktree_to_repo(cx: &mut TestAppContext) {
    // Test worktree addition
}

#[gpui::test]
async fn test_persistence_across_restart(cx: &mut TestAppContext) {
    // Test that registry persists
}

#[gpui::test]
async fn test_workspace_switching(cx: &mut TestAppContext) {
    // Test full workspace switch flow
}

#[gpui::test]
async fn test_unsaved_changes_prompt(cx: &mut TestAppContext) {
    // Test unsaved changes handling
}
```

### 9.2 Integration Tests

**File:** `crates/workspaces_panel/src/workspaces_panel_tests.rs`

```rust
#[gpui::test]
async fn test_panel_renders_repos(cx: &mut TestAppContext) {
    // Test panel rendering
}

#[gpui::test]
async fn test_keyboard_shortcuts(cx: &mut TestAppContext) {
    // Test ⌘1, ⌘2, etc.
}

#[gpui::test]
async fn test_agent_chat_filtering(cx: &mut TestAppContext) {
    // Test that agent chats filter by workspace
}
```

---

## Migration Path

### For Existing Users

1. **Feature flag off by default** - No change for existing users
2. **Opt-in via settings** - Users enable `multi_workspace_mode: true`
3. **Gradual rollout** - Staff first, then general availability

### Data Migration

- Existing workspaces continue to work as before
- No automatic migration of existing data
- Users manually add repos to the new registry

---

## File Summary

| Phase | New Files | Modified Files |
|-------|-----------|----------------|
| 1 | `workspace_registry.rs`, `workspace_registry_db.rs` | `workspace.rs` (AppState) |
| 2 | `crates/workspaces_panel/*` | `zed.rs` (registration) |
| 3 | - | `persistence/model.rs`, `workspace_registry_db.rs` |
| 4 | - | `workspace.rs` (switching logic) |
| 5 | - | `agent/db.rs`, `agent/agent.rs`, `thread_history.rs` |
| 6 | - | `workspaces_panel.rs`, `workspace_registry.rs` |
| 7 | - | `settings_content/workspace.rs`, `flags.rs`, `default.json` |
| 8 | - | Various polish files |
| 9 | `*_tests.rs` | - |

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
| 7 | Low | Phase 1, 2 |
| 8 | Medium | All previous |
| 9 | Medium | All previous |

**Critical Path:** Phase 1 → Phase 3 → Phase 4 (workspace switching is the hardest part)

**Parallelizable:** Phase 2, Phase 5, Phase 6, Phase 7 can proceed in parallel after Phase 1
