//! Shell commands, transient state, and keyboard/pointer parity.

use crate::activity::SurfaceId;
use crate::components::NodeId;
use crate::layout::LayoutMode;
use crate::theme::{Density, TextScale, ThemeConfig, ThemeName};
use serde::{Deserialize, Serialize};

/// Commands available from a pointer, keyboard, or launcher result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Command {
    /// Open/close the launcher (Command/Super-Space or click).
    ToggleLauncher,
    /// Cycle master, grid, and single layouts.
    CycleLayout,
    /// Enter focus mode for a surface.
    FocusSurface(SurfaceId),
    /// Focus a stable semantic node inside a surface.
    FocusNode {
        /// Stable surface target.
        surface_id: SurfaceId,
        /// Stable semantic node target.
        node_id: NodeId,
    },
    /// Restore the layout before focus mode.
    RestoreLayout,
    /// Open/close notifications.
    ToggleNotifications,
    /// Open/close the Theme Lab.
    ToggleThemeLab,
    /// Switch the current activity.
    SelectActivity(String),
    /// Set the palette.
    SetTheme(ThemeName),
    /// Set spacing/control density.
    SetDensity(Density),
    /// Set text scale.
    SetScale(TextScale),
    /// Toggle reduced motion.
    SetReducedMotion(bool),
    /// Move the desktop splitter by a percentage-point delta.
    ResizeMaster(i8),
}

/// User-visible state for the shell fixture.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShellState {
    /// Selected activity id.
    pub activity_id: String,
    /// Current layout mode.
    pub layout: LayoutMode,
    /// Layout mode to restore after focus.
    pub previous_layout: Option<LayoutMode>,
    /// Stable surface/node focus target.
    pub focus: Option<FocusTarget>,
    /// Launcher visibility.
    pub launcher_open: bool,
    /// Notification panel visibility.
    pub notifications_open: bool,
    /// Theme Lab visibility.
    pub theme_lab_open: bool,
    /// Desktop master-column percentage.
    pub master_percent: u8,
    /// Theme and accessibility settings.
    pub theme: ThemeConfig,
}

/// Typed focus target used by layout, reconciliation, and display.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FocusTarget {
    /// Stable surface target.
    pub surface_id: SurfaceId,
    /// Optional stable semantic node target.
    pub node_id: Option<NodeId>,
}

impl Default for ShellState {
    fn default() -> Self {
        Self {
            activity_id: "cats".to_owned(),
            layout: LayoutMode::Master,
            previous_layout: None,
            focus: None,
            launcher_open: false,
            notifications_open: false,
            theme_lab_open: false,
            master_percent: crate::layout::MASTER_DEFAULT_PERCENT,
            theme: ThemeConfig::default(),
        }
    }
}

impl ShellState {
    /// Apply a command and return whether visible state changed.
    pub fn apply(&mut self, command: Command) -> bool {
        let before = self.clone();
        match command {
            Command::ToggleLauncher => self.launcher_open = !self.launcher_open,
            Command::CycleLayout => {
                self.layout = match self.layout {
                    LayoutMode::Master => LayoutMode::Grid,
                    LayoutMode::Grid => LayoutMode::Single,
                    LayoutMode::Single | LayoutMode::Focus => LayoutMode::Master,
                };
                if self.layout != LayoutMode::Focus {
                    self.previous_layout = None;
                }
            }
            Command::FocusSurface(surface_id) => {
                self.previous_layout = Some(self.layout);
                self.layout = LayoutMode::Focus;
                self.focus = Some(FocusTarget {
                    surface_id,
                    node_id: None,
                });
            }
            Command::FocusNode {
                surface_id,
                node_id,
            } => {
                self.previous_layout = Some(self.layout);
                self.layout = LayoutMode::Focus;
                self.focus = Some(FocusTarget {
                    surface_id,
                    node_id: Some(node_id),
                });
            }
            Command::RestoreLayout => {
                if let Some(previous) = self.previous_layout.take() {
                    self.layout = previous;
                } else {
                    self.layout = LayoutMode::Master;
                }
            }
            Command::ToggleNotifications => self.notifications_open = !self.notifications_open,
            Command::ToggleThemeLab => self.theme_lab_open = !self.theme_lab_open,
            Command::SelectActivity(activity) => self.activity_id = activity,
            Command::SetTheme(name) => self.theme.name = name,
            Command::SetDensity(density) => self.theme.density = density,
            Command::SetScale(scale) => self.theme.scale = scale,
            Command::SetReducedMotion(value) => self.theme.reduced_motion = value,
            Command::ResizeMaster(delta) => {
                let current = i16::from(self.master_percent);
                self.master_percent = (current + i16::from(delta)).clamp(
                    i16::from(crate::layout::MASTER_MIN_PERCENT),
                    i16::from(crate::layout::MASTER_MAX_PERCENT),
                ) as u8;
            }
        }
        *self != before
    }

    /// Resolve the current theme.
    pub fn resolved_theme(&self) -> crate::theme::Theme {
        crate::theme::Theme::resolve(self.theme)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_restores_layout_and_resize_clamps() {
        let mut state = ShellState::default();
        assert!(state.apply(Command::FocusSurface(SurfaceId::new("surface-1").unwrap())));
        assert_eq!(state.layout, LayoutMode::Focus);
        assert!(state.apply(Command::RestoreLayout));
        assert_eq!(state.layout, LayoutMode::Master);
        assert!(state.apply(Command::ResizeMaster(100)));
        assert_eq!(state.master_percent, crate::layout::MASTER_MAX_PERCENT);
    }
}
