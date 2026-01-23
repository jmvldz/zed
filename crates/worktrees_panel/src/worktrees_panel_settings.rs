use gpui::{Pixels, px};
use settings::RegisterSetting;
pub use settings::DockSide;

#[derive(Debug, Clone, Copy, PartialEq, RegisterSetting)]
pub struct WorktreesPanelSettings {
    pub button: bool,
    pub default_width: Pixels,
    pub dock: DockSide,
}

impl settings::Settings for WorktreesPanelSettings {
    fn from_settings(content: &settings::SettingsContent) -> Self {
        let panel = content.worktrees_panel.as_ref();
        Self {
            button: panel.and_then(|p| p.button).unwrap_or(true),
            default_width: panel
                .and_then(|p| p.default_width)
                .map(px)
                .unwrap_or(px(240.0)),
            dock: panel
                .and_then(|p| p.dock.clone())
                .unwrap_or(DockSide::Left),
        }
    }
}
