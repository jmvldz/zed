use chrono::{DateTime, Utc};
use collections::HashMap;
use gpui::{Context, Entity, EventEmitter, SharedString, Task, WeakEntity};
use project::Project;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use util::ResultExt;

const CACHED_PROJECT_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorktreeSlotId(pub String);

impl WorktreeSlotId {
    pub fn from_git_path(git_path: &Path) -> Self {
        Self(git_path.to_string_lossy().into_owned())
    }

    pub fn from_worktree_path(worktree_path: &Path) -> Self {
        Self(worktree_path.to_string_lossy().into_owned())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeEntry {
    pub slot_id: WorktreeSlotId,
    pub worktree_path: PathBuf,
    pub branch_name: SharedString,
    pub last_accessed: DateTime<Utc>,
    pub agent_chat_count: usize,
}

impl WorktreeEntry {
    pub fn new(slot_id: WorktreeSlotId, worktree_path: PathBuf, branch_name: SharedString) -> Self {
        Self {
            slot_id,
            worktree_path,
            branch_name,
            last_accessed: Utc::now(),
            agent_chat_count: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotState {
    Active,
    Cached,
    Unloaded,
}

pub struct WorktreeSlot {
    pub slot_id: WorktreeSlotId,
    pub worktree_path: PathBuf,
    pub state: SlotState,
    pub project: Option<Entity<Project>>,
    pub serialized: Option<SerializedWorkspaceSlot>,
    cleanup_task: Option<Task<()>>,
}

impl WorktreeSlot {
    pub fn new(slot_id: WorktreeSlotId, worktree_path: PathBuf) -> Self {
        Self {
            slot_id,
            worktree_path,
            state: SlotState::Unloaded,
            project: None,
            serialized: None,
            cleanup_task: None,
        }
    }

    pub fn is_loaded(&self) -> bool {
        self.project.is_some()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedWorkspaceSlot {
    pub slot_id: WorktreeSlotId,
}

pub enum WorktreeRegistryEvent {
    ActiveSlotChanged {
        old_slot_id: Option<WorktreeSlotId>,
        new_slot_id: WorktreeSlotId,
    },
    WorktreeAdded(WorktreeSlotId),
    WorktreeRemoved(WorktreeSlotId),
    WorktreesScanned,
}

pub struct WorktreeRegistry {
    repo_identity_path: PathBuf,
    repo_root_path: PathBuf,
    worktrees: Vec<WorktreeEntry>,
    slots: HashMap<WorktreeSlotId, WorktreeSlot>,
    active_slot_id: Option<WorktreeSlotId>,
    project: WeakEntity<Project>,
    is_git_repo: bool,
    _scan_task: Option<Task<()>>,
}

impl EventEmitter<WorktreeRegistryEvent> for WorktreeRegistry {}

impl WorktreeRegistry {
    pub fn new(
        project: WeakEntity<Project>,
        repo_root_path: PathBuf,
        repo_identity_path: PathBuf,
        is_git_repo: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        log::info!(
            "WorktreeRegistry::new repo_root_path={:?} repo_identity_path={:?} is_git_repo={}",
            repo_root_path,
            repo_identity_path,
            is_git_repo
        );
        let mut registry = Self {
            repo_identity_path,
            repo_root_path: repo_root_path.clone(),
            worktrees: Vec::new(),
            slots: HashMap::default(),
            active_slot_id: None,
            project,
            is_git_repo,
            _scan_task: None,
        };

        let initial_slot_id = WorktreeSlotId::from_worktree_path(&repo_root_path);
        let initial_entry = WorktreeEntry::new(
            initial_slot_id.clone(),
            repo_root_path.clone(),
            if is_git_repo {
                "(scanning...)".into()
            } else {
                "(no git)".into()
            },
        );
        registry.worktrees.push(initial_entry);
        registry.slots.insert(
            initial_slot_id.clone(),
            WorktreeSlot::new(initial_slot_id.clone(), repo_root_path),
        );
        registry.active_slot_id = Some(initial_slot_id);

        if is_git_repo {
            registry.scan_repo_worktrees(cx);
        }

        registry
    }

    pub fn repo_identity_path(&self) -> &Path {
        &self.repo_identity_path
    }

    pub fn repo_root_path(&self) -> &Path {
        &self.repo_root_path
    }

    pub fn is_git_repo(&self) -> bool {
        self.is_git_repo
    }

    pub fn enable_git_repo(&mut self, repo_identity_path: PathBuf, cx: &mut Context<Self>) {
        if self.is_git_repo {
            if self.repo_identity_path != repo_identity_path {
                self.repo_identity_path = repo_identity_path;
            }
            return;
        }

        self.is_git_repo = true;
        self.repo_identity_path = repo_identity_path;

        let mut updated = false;
        for entry in &mut self.worktrees {
            if entry.worktree_path == self.repo_root_path && entry.branch_name == "(no git)" {
                entry.branch_name = "(scanning...)".into();
                updated = true;
            }
        }

        if updated {
            cx.notify();
        }

        self.scan_repo_worktrees(cx);
    }

    pub fn worktrees(&self) -> &[WorktreeEntry] {
        &self.worktrees
    }

    pub fn active_slot_id(&self) -> Option<&WorktreeSlotId> {
        self.active_slot_id.as_ref()
    }

    pub fn active_worktree(&self) -> Option<&WorktreeEntry> {
        self.active_slot_id
            .as_ref()
            .and_then(|id| self.worktrees.iter().find(|w| &w.slot_id == id))
    }

    pub fn get_slot(&self, slot_id: &WorktreeSlotId) -> Option<&WorktreeSlot> {
        self.slots.get(slot_id)
    }

    pub fn get_slot_mut(&mut self, slot_id: &WorktreeSlotId) -> Option<&mut WorktreeSlot> {
        self.slots.get_mut(slot_id)
    }

    pub fn set_active_slot(&mut self, slot_id: WorktreeSlotId, cx: &mut Context<Self>) {
        let old_slot_id = self.active_slot_id.clone();

        if let Some(old_id) = &old_slot_id {
            if let Some(slot) = self.slots.get_mut(old_id) {
                if slot.state == SlotState::Active {
                    slot.state = SlotState::Cached;
                    self.schedule_slot_cleanup(old_id.clone(), cx);
                }
            }
        }

        if let Some(entry) = self.worktrees.iter_mut().find(|w| w.slot_id == slot_id) {
            entry.last_accessed = Utc::now();
        }

        if let Some(slot) = self.slots.get_mut(&slot_id) {
            slot.state = SlotState::Active;
            slot.cleanup_task = None;
        }

        self.active_slot_id = Some(slot_id.clone());

        cx.emit(WorktreeRegistryEvent::ActiveSlotChanged {
            old_slot_id,
            new_slot_id: slot_id,
        });
        cx.notify();
    }

    fn schedule_slot_cleanup(&mut self, slot_id: WorktreeSlotId, cx: &mut Context<Self>) {
        if let Some(slot) = self.slots.get_mut(&slot_id) {
            let task = cx.spawn({
                let slot_id = slot_id.clone();
                async move |this, cx| {
                    cx.background_executor()
                        .timer(CACHED_PROJECT_TIMEOUT)
                        .await;
                    this.update(cx, |this, cx| {
                        this.cleanup_cached_slot(&slot_id, cx);
                    })
                    .log_err();
                }
            });
            slot.cleanup_task = Some(task);
        }
    }

    fn cleanup_cached_slot(&mut self, slot_id: &WorktreeSlotId, cx: &mut Context<Self>) {
        if let Some(slot) = self.slots.get_mut(slot_id) {
            if slot.state == SlotState::Cached {
                slot.project = None;
                slot.state = SlotState::Unloaded;
                cx.notify();
            }
        }
    }

    pub fn add_worktree(&mut self, entry: WorktreeEntry, cx: &mut Context<Self>) {
        let slot_id = entry.slot_id.clone();
        let worktree_path = entry.worktree_path.clone();

        if !self.worktrees.iter().any(|w| w.slot_id == slot_id) {
            self.worktrees.push(entry);
            self.slots.insert(
                slot_id.clone(),
                WorktreeSlot::new(slot_id.clone(), worktree_path),
            );
            cx.emit(WorktreeRegistryEvent::WorktreeAdded(slot_id));
            cx.notify();
        }
    }

    pub fn remove_worktree(&mut self, slot_id: &WorktreeSlotId, cx: &mut Context<Self>) {
        if Some(slot_id) == self.active_slot_id.as_ref() {
            return;
        }

        self.worktrees.retain(|w| &w.slot_id != slot_id);
        self.slots.remove(slot_id);
        cx.emit(WorktreeRegistryEvent::WorktreeRemoved(slot_id.clone()));
        cx.notify();
    }

    pub fn scan_repo_worktrees(&mut self, cx: &mut Context<Self>) {
        if !self.is_git_repo {
            log::info!(
                "WorktreeRegistry::scan_repo_worktrees skipped (not a git repo) repo_root_path={:?}",
                self.repo_root_path
            );
            return;
        }

        let Some(project) = self.project.upgrade() else {
            log::info!(
                "WorktreeRegistry::scan_repo_worktrees skipped (project missing) repo_root_path={:?}",
                self.repo_root_path
            );
            return;
        };

        let git_store = project.read(cx).git_store().clone();
        let repo = git_store
            .read(cx)
            .repositories()
            .values()
            .next()
            .cloned();

        let Some(repo) = repo else {
            log::info!(
                "WorktreeRegistry::scan_repo_worktrees skipped (no repositories) repo_root_path={:?}",
                self.repo_root_path
            );
            return;
        };

        let receiver = repo.update(cx, |repo, _| repo.worktrees());

        let task = cx.spawn(async move |this, cx| {
            if let Ok(Ok(worktrees)) = receiver.await {
                this.update(cx, |this, cx| {
                    this.update_worktrees_from_scan(worktrees, cx);
                })
                .log_err();
            }
        });

        self._scan_task = Some(task);
    }

    fn update_worktrees_from_scan(
        &mut self,
        git_worktrees: Vec<git::repository::Worktree>,
        cx: &mut Context<Self>,
    ) {
        log::info!(
            "WorktreeRegistry::update_worktrees_from_scan repo_root_path={:?} worktrees={}",
            self.repo_root_path,
            git_worktrees.len()
        );
        let mut seen_slot_ids = std::collections::HashSet::new();

        for git_worktree in git_worktrees {
            let slot_id = WorktreeSlotId::from_worktree_path(&git_worktree.path);
            seen_slot_ids.insert(slot_id.clone());

            let branch_name: SharedString = git_worktree.branch().to_string().into();

            if let Some(entry) = self.worktrees.iter_mut().find(|w| w.slot_id == slot_id) {
                entry.branch_name = branch_name;
                entry.worktree_path = git_worktree.path.clone();
            } else {
                let entry =
                    WorktreeEntry::new(slot_id.clone(), git_worktree.path.clone(), branch_name);
                self.worktrees.push(entry);
                self.slots.insert(
                    slot_id.clone(),
                    WorktreeSlot::new(slot_id, git_worktree.path),
                );
            }
        }

        self.worktrees.retain(|w| {
            seen_slot_ids.contains(&w.slot_id)
                || self.active_slot_id.as_ref() == Some(&w.slot_id)
        });

        self.sort_worktrees();
        cx.emit(WorktreeRegistryEvent::WorktreesScanned);
        cx.notify();
    }

    fn sort_worktrees(&mut self) {
        let active_slot_id = self.active_slot_id.clone();
        self.worktrees.sort_by(|a, b| {
            let a_active = active_slot_id.as_ref() == Some(&a.slot_id);
            let b_active = active_slot_id.as_ref() == Some(&b.slot_id);

            match (a_active, b_active) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a
                    .branch_name
                    .cmp(&b.branch_name)
                    .then_with(|| a.worktree_path.cmp(&b.worktree_path)),
            }
        });
    }

    pub fn update_agent_chat_count(
        &mut self,
        slot_id: &WorktreeSlotId,
        count: usize,
        cx: &mut Context<Self>,
    ) {
        if let Some(entry) = self.worktrees.iter_mut().find(|w| &w.slot_id == slot_id) {
            entry.agent_chat_count = count;
            cx.notify();
        }
    }

    pub fn validate_worktree_paths(&mut self, cx: &mut Context<Self>) {
        let to_remove: Vec<_> = self
            .worktrees
            .iter()
            .filter(|w| {
                !w.worktree_path.exists() && self.active_slot_id.as_ref() != Some(&w.slot_id)
            })
            .map(|w| w.slot_id.clone())
            .collect();

        for slot_id in to_remove {
            self.remove_worktree(&slot_id, cx);
        }
    }
}

pub fn derive_repo_identity_path(
    repo_root_path: &Path,
    git_common_dir: Option<&Path>,
) -> PathBuf {
    git_common_dir
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| repo_root_path.to_path_buf())
}
