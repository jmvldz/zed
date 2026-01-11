use std::path::PathBuf;
use std::sync::Arc;

use gpui::{
    div, App, Context, Entity, EventEmitter, FocusHandle, Focusable,
    IntoElement, ParentElement, Render, Styled, Task, Window,
};
use picker::{Picker, PickerDelegate};
use ui::{ListItem, ListItemSpacing, prelude::*};

#[derive(Clone, Debug)]
pub struct Commit {
    pub sha: String,
    pub author: String,
    pub date: String,
    pub subject: String,
}

pub enum CommitPickerEvent {
    Selected(Vec<Commit>),
    Dismissed,
}

pub struct CommitPicker {
    picker: Entity<Picker<CommitPickerDelegate>>,
}

impl CommitPicker {
    pub fn new(
        repo_path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let delegate = CommitPickerDelegate::new(repo_path);
        let picker = cx.new(|cx| Picker::uniform_list(delegate, window, cx));

        Self { picker }
    }
}

impl EventEmitter<CommitPickerEvent> for CommitPicker {}

impl Focusable for CommitPicker {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.picker.focus_handle(cx)
    }
}

impl Render for CommitPicker {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.colors().elevated_surface_background)
            .rounded_lg()
            .shadow_lg()
            .child(self.picker.clone())
    }
}

pub struct CommitPickerDelegate {
    repo_path: PathBuf,
    commits: Vec<Commit>,
    filtered_commits: Vec<usize>,
    selected_index: usize,
    selected_commits: Vec<String>,
}

impl CommitPickerDelegate {
    pub fn new(repo_path: PathBuf) -> Self {
        Self {
            repo_path,
            commits: Vec::new(),
            filtered_commits: Vec::new(),
            selected_index: 0,
            selected_commits: Vec::new(),
        }
    }

    fn load_commits(&mut self) {
        self.commits.clear();
        self.filtered_commits.clear();

        let output = std::process::Command::new("git")
            .args([
                "log",
                "--format=%H|%an|%ar|%s",
                "-50",
            ])
            .current_dir(&self.repo_path)
            .output();

        if let Ok(output) = output {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for (idx, line) in stdout.lines().enumerate() {
                    let parts: Vec<&str> = line.splitn(4, '|').collect();
                    if parts.len() == 4 {
                        self.commits.push(Commit {
                            sha: parts[0].to_string(),
                            author: parts[1].to_string(),
                            date: parts[2].to_string(),
                            subject: parts[3].to_string(),
                        });
                        self.filtered_commits.push(idx);
                    }
                }
            }
        }
    }

    fn toggle_selection(&mut self, sha: &str) {
        if let Some(pos) = self.selected_commits.iter().position(|s| s == sha) {
            self.selected_commits.remove(pos);
        } else {
            self.selected_commits.push(sha.to_string());
        }
    }

    pub fn selected_commits(&self) -> Vec<Commit> {
        self.commits
            .iter()
            .filter(|c| self.selected_commits.contains(&c.sha))
            .cloned()
            .collect()
    }
}

impl PickerDelegate for CommitPickerDelegate {
    type ListItem = ListItem;

    fn match_count(&self) -> usize {
        self.filtered_commits.len()
    }

    fn selected_index(&self) -> usize {
        self.selected_index
    }

    fn set_selected_index(
        &mut self,
        ix: usize,
        _window: &mut Window,
        _cx: &mut Context<Picker<Self>>,
    ) {
        self.selected_index = ix;
    }

    fn placeholder_text(&self, _window: &mut Window, _cx: &mut App) -> Arc<str> {
        "Search commits...".into()
    }

    fn update_matches(
        &mut self,
        query: String,
        _window: &mut Window,
        _cx: &mut Context<Picker<Self>>,
    ) -> Task<()> {
        if self.commits.is_empty() {
            self.load_commits();
        }

        self.filtered_commits.clear();

        let query_lower = query.to_lowercase();

        for (idx, commit) in self.commits.iter().enumerate() {
            if query.is_empty()
                || commit.sha.to_lowercase().contains(&query_lower)
                || commit.subject.to_lowercase().contains(&query_lower)
                || commit.author.to_lowercase().contains(&query_lower)
            {
                self.filtered_commits.push(idx);
            }
        }

        self.selected_index = 0;

        Task::ready(())
    }

    fn confirm(&mut self, _secondary: bool, _window: &mut Window, _cx: &mut Context<Picker<Self>>) {
        let sha = self.filtered_commits.get(self.selected_index)
            .and_then(|&idx| self.commits.get(idx))
            .map(|c| c.sha.clone());
        if let Some(sha) = sha {
            self.toggle_selection(&sha);
        }
    }

    fn dismissed(&mut self, _window: &mut Window, _cx: &mut Context<Picker<Self>>) {
        // Picker was dismissed
    }

    fn render_match(
        &self,
        ix: usize,
        selected: bool,
        _window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) -> Option<Self::ListItem> {
        let &commit_idx = self.filtered_commits.get(ix)?;
        let commit = self.commits.get(commit_idx)?;
        let is_checked = self.selected_commits.contains(&commit.sha);
        let theme = cx.theme();

        let short_sha = &commit.sha[..7.min(commit.sha.len())];

        Some(
            ListItem::new(ix)
                .inset(true)
                .spacing(ListItemSpacing::Sparse)
                .toggle_state(selected)
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .size_4()
                                .rounded_sm()
                                .border_1()
                                .border_color(theme.colors().border)
                                .when(is_checked, |div| div.bg(theme.colors().element_selected))
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_0p5()
                                .child(
                                    div()
                                        .flex()
                                        .flex_row()
                                        .gap_2()
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(theme.colors().text_accent)
                                                .child(short_sha.to_string())
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(theme.colors().text_muted)
                                                .child(commit.date.clone())
                                        )
                                )
                                .child(commit.subject.clone())
                        )
                ),
        )
    }
}
