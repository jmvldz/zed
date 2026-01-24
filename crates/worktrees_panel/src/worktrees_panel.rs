mod worktrees_panel_settings;

use chrono::{DateTime, Utc};
use gpui::{
    actions, div, prelude::*, Action, App, AsyncWindowContext, Context, DismissEvent, Entity,
    EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement, ParentElement, Pixels,
    Render, Styled, Subscription, Task, WeakEntity, Window,
};
use settings::Settings;
use ui::{prelude::*, ListItem, ListItemSpacing};
use std::{collections::HashSet, path::PathBuf};
use util::ResultExt;
use workspace::{
    CloseIntent, OpenOptions, Workspace, WorktreeRegistryEvent, WorktreeSlotId, open_paths,
    dock::{DockPosition, Panel, PanelEvent},
};
use worktrees_panel_settings::{DockSide, WorktreesPanelSettings};

actions!(
    worktrees_panel,
    [
        ToggleFocus,
        SwitchToWorktree1,
        SwitchToWorktree2,
        SwitchToWorktree3,
        SwitchToWorktree4,
        SwitchToWorktree5,
    ]
);

const WORKTREES_PANEL_KEY: &str = "WorktreesPanel";

pub fn init(cx: &mut App) {
    WorktreesPanelSettings::register(cx);

    cx.observe_new::<Workspace>(|workspace, _window, _cx| {
        workspace
            .register_action(|workspace, _: &ToggleFocus, window, cx| {
                workspace.toggle_panel_focus::<WorktreesPanel>(window, cx);
            })
            .register_action(|workspace, _: &SwitchToWorktree1, window, cx| {
                switch_to_worktree_at_index(workspace, 0, window, cx);
            })
            .register_action(|workspace, _: &SwitchToWorktree2, window, cx| {
                switch_to_worktree_at_index(workspace, 1, window, cx);
            })
            .register_action(|workspace, _: &SwitchToWorktree3, window, cx| {
                switch_to_worktree_at_index(workspace, 2, window, cx);
            })
            .register_action(|workspace, _: &SwitchToWorktree4, window, cx| {
                switch_to_worktree_at_index(workspace, 3, window, cx);
            })
            .register_action(|workspace, _: &SwitchToWorktree5, window, cx| {
                switch_to_worktree_at_index(workspace, 4, window, cx);
            });
    })
    .detach();
}

fn switch_to_worktree_at_index(
    workspace: &mut Workspace,
    index: usize,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let Some(registry) = workspace.worktree_registry().cloned() else {
        return;
    };

    let worktree_path = registry.read(cx).worktrees().get(index).map(|w| w.worktree_path.clone());

    let Some(worktree_path) = worktree_path else {
        return;
    };

    replace_workspace_root(worktree_path, workspace, window, cx);
}

fn replace_workspace_root(
    worktree_path: PathBuf,
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let old_worktree_id = workspace
        .worktree_registry()
        .and_then(|registry| registry.read(cx).active_worktree())
        .and_then(|entry| {
            workspace
                .project()
                .read(cx)
                .visible_worktrees(cx)
                .find(|worktree| worktree.read(cx).abs_path().as_ref() == entry.worktree_path.as_path())
                .map(|worktree| worktree.read(cx).id())
        });

    let mut open_rel_paths = Vec::new();
    for item in workspace.items(cx) {
        if let Some(project_path) = item.project_path(cx)
            && Some(project_path.worktree_id) == old_worktree_id
        {
            open_rel_paths.push(project_path.path.clone());
        }
    }

    let app_state = workspace.app_state().clone();
    let window_handle = window.window_handle().downcast::<Workspace>();
    let prepare = workspace.prepare_to_close(CloseIntent::ReplaceWindow, window, cx);

    cx.spawn_in(window, async move |_, cx| {
        if !prepare.await? {
            return anyhow::Ok(());
        }

        let fs = app_state.fs.clone();
        let mut paths_to_open = Vec::new();
        paths_to_open.push(worktree_path.clone());

        let mut seen = HashSet::new();
        for rel_path in open_rel_paths {
            let candidate = worktree_path.join(rel_path.as_unix_str());
            if !seen.insert(candidate.clone()) {
                continue;
            }
            let Some(metadata) = fs.metadata(&candidate).await.log_err().flatten() else {
                continue;
            };
            if metadata.is_dir {
                continue;
            }
            paths_to_open.push(candidate);
        }

        let open_task = cx.update(|_window, cx| {
            open_paths(
                &paths_to_open,
                app_state,
                OpenOptions {
                    open_new_workspace: Some(true),
                    replace_window: window_handle,
                    ..Default::default()
                },
                cx,
            )
        })?;

        let _ = open_task.await;

        anyhow::Ok(())
    })
    .detach_and_log_err(cx);
}

pub struct WorktreesPanel {
    workspace: WeakEntity<Workspace>,
    focus_handle: FocusHandle,
    width: Option<Pixels>,
    _subscriptions: Vec<Subscription>,
}

impl WorktreesPanel {
    pub fn new(workspace: &Workspace, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        let weak_workspace = workspace.weak_handle();

        let mut subscriptions = Vec::new();

        if let Some(registry) = workspace.worktree_registry() {
            subscriptions.push(cx.subscribe(registry, |_this, _, event, cx| {
                match event {
                    WorktreeRegistryEvent::ActiveSlotChanged { .. }
                    | WorktreeRegistryEvent::WorktreeAdded(_)
                    | WorktreeRegistryEvent::WorktreeRemoved(_)
                    | WorktreeRegistryEvent::WorktreesScanned => {
                        cx.notify();
                    }
                }
            }));
        }

        Self {
            workspace: weak_workspace,
            focus_handle,
            width: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn load(
        workspace: WeakEntity<Workspace>,
        cx: &mut AsyncWindowContext,
    ) -> Task<anyhow::Result<Entity<Self>>> {
        cx.spawn(async move |cx| {
            workspace.update_in(cx, |workspace, window, cx| {
                cx.new(|cx| WorktreesPanel::new(workspace, window, cx))
            })
        })
    }

    #[allow(dead_code)]
    fn switch_to_worktree(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };

        let slot_id = workspace.read(cx).worktree_registry().and_then(|registry| {
            registry
                .read(cx)
                .worktrees()
                .get(index)
                .map(|entry| entry.slot_id.clone())
        });

        if let Some(slot_id) = slot_id {
            self.switch_to_slot(slot_id, window, cx);
        }
    }

    fn switch_to_slot(
        &mut self,
        slot_id: WorktreeSlotId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };

        let worktree_path: Option<PathBuf> = workspace.read(cx).worktree_registry().and_then(|registry| {
            let registry = registry.read(cx);
            registry
                .worktrees()
                .iter()
                .find(|w| w.slot_id == slot_id)
                .map(|w| w.worktree_path.clone())
        });

        let Some(worktree_path) = worktree_path else {
            return;
        };

        workspace.update(cx, |workspace, cx| {
            replace_workspace_root(worktree_path, workspace, window, cx);
        });
    }
}

fn format_last_accessed(last_accessed: DateTime<Utc>) -> String {
    let now = Utc::now();
    let duration = now.signed_duration_since(last_accessed);

    if duration.num_seconds() < 60 {
        "just now".to_string()
    } else if duration.num_minutes() < 60 {
        let minutes = duration.num_minutes();
        format!("{}m ago", minutes)
    } else if duration.num_hours() < 24 {
        let hours = duration.num_hours();
        format!("{}h ago", hours)
    } else {
        let days = duration.num_days();
        format!("{}d ago", days)
    }
}

impl EventEmitter<PanelEvent> for WorktreesPanel {}
impl EventEmitter<DismissEvent> for WorktreesPanel {}

impl Focusable for WorktreesPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Panel for WorktreesPanel {
    fn persistent_name() -> &'static str {
        "Worktrees Panel"
    }

    fn panel_key() -> &'static str {
        WORKTREES_PANEL_KEY
    }

    fn position(&self, _window: &Window, cx: &App) -> DockPosition {
        match WorktreesPanelSettings::get_global(cx).dock {
            DockSide::Left => DockPosition::Left,
            DockSide::Right => DockPosition::Right,
        }
    }

    fn position_is_valid(&self, position: DockPosition) -> bool {
        matches!(position, DockPosition::Left | DockPosition::Right)
    }

    fn set_position(&mut self, _position: DockPosition, _window: &mut Window, _cx: &mut Context<Self>) {
    }

    fn size(&self, _window: &Window, cx: &App) -> Pixels {
        self.width
            .unwrap_or_else(|| WorktreesPanelSettings::get_global(cx).default_width)
    }

    fn set_size(&mut self, size: Option<Pixels>, _window: &mut Window, cx: &mut Context<Self>) {
        self.width = size;
        cx.notify();
    }

    fn icon(&self, _window: &Window, cx: &App) -> Option<IconName> {
        WorktreesPanelSettings::get_global(cx)
            .button
            .then_some(IconName::FileTree)
    }

    fn icon_tooltip(&self, _window: &Window, _cx: &App) -> Option<&'static str> {
        Some("Worktrees Panel")
    }

    fn toggle_action(&self) -> Box<dyn Action> {
        Box::new(ToggleFocus)
    }

    fn activation_priority(&self) -> u32 {
        3
    }
}

impl Render for WorktreesPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(workspace) = self.workspace.upgrade() else {
            return div().into_any_element();
        };

        let repo_name = workspace
            .read(cx)
            .worktree_registry()
            .map(|r| {
                r.read(cx)
                    .repo_root_path()
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "Workspace".to_string())
            })
            .unwrap_or_else(|| "Workspace".to_string());

        let header = h_flex()
            .px_2()
            .py_1()
            .justify_between()
            .child(
                div()
                    .text_sm()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(repo_name),
            );

        let worktree_items = {
            let Some(registry) = workspace.read(cx).worktree_registry() else {
                return v_flex()
                    .id("worktrees-panel")
                    .key_context("WorktreesPanel")
                    .track_focus(&self.focus_handle)
                    .size_full()
                    .overflow_hidden()
                    .bg(cx.theme().colors().panel_background)
                    .child(header)
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .child("No worktree registry available"),
                    )
                    .into_any_element();
            };

            let registry = registry.read(cx);
            let worktrees = registry.worktrees();
            let active_slot_id = registry.active_slot_id().cloned();

            if worktrees.is_empty() {
                v_flex().px_2().py_1().child("No worktrees found")
            } else {
                v_flex().children(
                    worktrees
                        .iter()
                        .enumerate()
                        .map(|(index, entry)| {
                            let is_active = active_slot_id.as_ref() == Some(&entry.slot_id);
                            let slot_id = entry.slot_id.clone();
                            let branch_name = entry.branch_name.clone();
                            let chat_count = entry.agent_chat_count;
                            let last_accessed = entry.last_accessed;

                            ListItem::new(ElementId::Name(format!("worktree-{}", index).into()))
                                .spacing(ListItemSpacing::Dense)
                                .toggle_state(is_active)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.switch_to_slot(slot_id.clone(), window, cx);
                                }))
                                .child(
                                    v_flex()
                                        .child(
                                            h_flex()
                                                .gap_2()
                                                .child(
                                                    Label::new(branch_name)
                                                        .weight(if is_active {
                                                            gpui::FontWeight::BOLD
                                                        } else {
                                                            gpui::FontWeight::NORMAL
                                                        }),
                                                )
                                                .when(chat_count > 0, |el| {
                                                    el.child(
                                                        Label::new(format!("({} chats)", chat_count))
                                                            .size(LabelSize::Small)
                                                            .color(Color::Muted),
                                                    )
                                                }),
                                        )
                                        .child(
                                            Label::new(format_last_accessed(last_accessed))
                                                .size(LabelSize::Small)
                                                .color(Color::Muted),
                                        ),
                                )
                        }),
                )
            }
        };

        v_flex()
            .id("worktrees-panel")
            .key_context("WorktreesPanel")
            .track_focus(&self.focus_handle)
            .size_full()
            .overflow_hidden()
            .bg(cx.theme().colors().panel_background)
            .child(header)
            .child(div().flex_1().child(worktree_items))
            .into_any_element()
    }
}
