use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use collections::HashMap;
use fuzzy::{StringMatch, StringMatchCandidate};
use gpui::{
    div, App, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    IntoElement, ParentElement, Render, Styled, Task, Window,
};
use picker::{Picker, PickerDelegate};
use smol::unblock;
use ui::{ListItem, ListItemSpacing, prelude::*};

pub enum FilePickerEvent {
    Selected(Vec<PathBuf>),
    Dismissed,
}

pub struct FilePicker {
    picker: Entity<Picker<FilePickerDelegate>>,
}

impl FilePicker {
    pub fn new(
        root_path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let delegate = FilePickerDelegate::new(root_path);
        let picker = cx.new(|cx| Picker::uniform_list(delegate, window, cx).modal(false));
        cx.subscribe(&picker, Self::handle_picker_dismissed).detach();

        Self { picker }
    }
}

impl FilePicker {
    fn handle_picker_dismissed(
        &mut self,
        _picker: Entity<Picker<FilePickerDelegate>>,
        _event: &DismissEvent,
        cx: &mut Context<Self>,
    ) {
        cx.emit(FilePickerEvent::Dismissed);
    }
}

impl EventEmitter<FilePickerEvent> for FilePicker {}

impl Focusable for FilePicker {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.picker.focus_handle(cx)
    }
}

impl Render for FilePicker {
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

pub struct FilePickerDelegate {
    root_path: PathBuf,
    files: Vec<PathBuf>,
    matches: Vec<StringMatch>,
    selected_index: usize,
    selected_files: HashMap<PathBuf, bool>,
    loading_files: bool,
}

impl FilePickerDelegate {
    pub fn new(root_path: PathBuf) -> Self {
        Self {
            root_path,
            files: Vec::new(),
            matches: Vec::new(),
            selected_index: 0,
            selected_files: HashMap::default(),
            loading_files: false,
        }
    }

    fn walk_directory(
        root_path: &PathBuf,
        dir: &PathBuf,
        files: &mut Vec<PathBuf>,
    ) -> Result<()> {
        let entries = std::fs::read_dir(dir)?;

        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = path.file_name().map(|n| n.to_string_lossy().to_string());

            if let Some(name) = &file_name {
                if name.starts_with('.') || Self::should_skip_directory(name) {
                    continue;
                }
            }

            if path.is_file() {
                if let Ok(relative) = path.strip_prefix(root_path) {
                    files.push(relative.to_path_buf());
                }
            } else if path.is_dir() {
                Self::walk_directory(root_path, &path, files)?;
            }
        }

        Ok(())
    }

    fn collect_files(root_path: &PathBuf) -> Result<Vec<PathBuf>> {
        let mut files = Vec::new();
        if root_path.exists() && root_path.is_dir() {
            Self::walk_directory(root_path, root_path, &mut files)?;
        }
        Ok(files)
    }

    fn should_skip_directory(name: &str) -> bool {
        matches!(
            name,
            "node_modules"
                | "target"
                | "build"
                | "dist"
                | ".git"
                | ".next"
                | "venv"
                | "__pycache__"
                | ".cache"
        )
    }

    fn toggle_selection(&mut self, path: &PathBuf) {
        let is_selected = self.selected_files.get(path).copied().unwrap_or(false);
        self.selected_files.insert(path.clone(), !is_selected);
    }

    pub fn selected_files(&self) -> Vec<PathBuf> {
        self.selected_files
            .iter()
            .filter(|(_, selected)| **selected)
            .map(|(path, _)| path.clone())
            .collect()
    }
}

impl PickerDelegate for FilePickerDelegate {
    type ListItem = ListItem;

    fn match_count(&self) -> usize {
        self.matches.len()
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
        "Search files...".into()
    }

    fn update_matches(
        &mut self,
        query: String,
        window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) -> Task<()> {
        if self.files.is_empty() {
            if self.loading_files {
                return Task::ready(());
            }

            self.loading_files = true;
            let root_path = self.root_path.clone();
            let query = query.clone();

            return cx.spawn_in(window, async move |picker, cx| {
                let files = unblock(move || FilePickerDelegate::collect_files(&root_path)).await;

                let (files, matches) = match files {
                    Ok(files) => {
                        let candidates: Vec<StringMatchCandidate> = files
                            .iter()
                            .enumerate()
                            .map(|(id, path)| StringMatchCandidate {
                                id,
                                string: path.to_string_lossy().to_string(),
                                char_bag: path.to_string_lossy().chars().collect(),
                            })
                            .collect();

                        let matches = if query.is_empty() {
                            candidates
                                .iter()
                                .map(|c| StringMatch {
                                    candidate_id: c.id,
                                    string: c.string.clone(),
                                    positions: Vec::new(),
                                    score: 0.0,
                                })
                                .collect()
                        } else {
                            fuzzy::match_strings(
                                &candidates,
                                &query,
                                false,
                                true,
                                100,
                                &Default::default(),
                                cx.background_executor().clone(),
                            )
                            .await
                        };

                        (files, matches)
                    }
                    Err(_) => (Vec::new(), Vec::new()),
                };

                picker
                    .update_in(cx, |picker, _window, cx| {
                        picker.delegate.loading_files = false;
                        picker.delegate.files = files;
                        picker.delegate.matches = matches;
                        picker.delegate.selected_index = 0;
                        cx.notify();
                    })
                    .ok();
            });
        }

        let candidates: Vec<StringMatchCandidate> = self
            .files
            .iter()
            .enumerate()
            .map(|(id, path)| StringMatchCandidate {
                id,
                string: path.to_string_lossy().to_string(),
                char_bag: path.to_string_lossy().chars().collect(),
            })
            .collect();

        let query = query.clone();
        cx.spawn_in(window, async move |picker, cx| {
            let matches = if query.is_empty() {
                candidates
                    .iter()
                    .map(|c| StringMatch {
                        candidate_id: c.id,
                        string: c.string.clone(),
                        positions: Vec::new(),
                        score: 0.0,
                    })
                    .collect()
            } else {
                fuzzy::match_strings(
                    &candidates,
                    &query,
                    false,
                    true,
                    100,
                    &Default::default(),
                    cx.background_executor().clone(),
                )
                .await
            };

            picker
                .update_in(cx, |picker, _window, _cx| {
                    picker.delegate.matches = matches;
                    picker.delegate.selected_index = 0;
                })
                .ok();
        })
    }

    fn confirm(&mut self, _secondary: bool, _window: &mut Window, _cx: &mut Context<Picker<Self>>) {
        let path = self.matches.get(self.selected_index)
            .and_then(|m| self.files.get(m.candidate_id))
            .cloned();
        if let Some(path) = path {
            self.toggle_selection(&path);
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
        let m = self.matches.get(ix)?;
        let path = self.files.get(m.candidate_id)?;
        let is_checked = self.selected_files.get(path).copied().unwrap_or(false);
        let theme = cx.theme();

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
                        .child(path.to_string_lossy().to_string())
                ),
        )
    }
}
