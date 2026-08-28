//! Shell commands, transient state, and keyboard/pointer parity.

use crate::activity::SurfaceId;
use crate::atlas::{AtlasInvocation, AtlasState, PlacementOrigin, PlacementTarget};
use crate::components::NodeId;
use crate::layout::LayoutMode;
use crate::theme::{Density, TextScale, ThemeConfig, ThemeName};
use serde::{Deserialize, Serialize};

/// Commands available from a pointer, keyboard, or launcher result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Command {
    /// Open/close the launcher (Command/Super-Space or click).
    ToggleLauncher,
    /// Open/close the Atlas from the control or a platform shortcut.
    ToggleAtlas(AtlasInvocation),
    /// Restore the selected Atlas activity while preserving its identity.
    ActivateAtlasTile(String),
    /// Begin pointer or keyboard placement for a stable activity.
    BeginAtlasPlacement {
        /// Stable activity identity.
        activity_id: String,
        /// Interaction origin.
        origin: PlacementOrigin,
    },
    /// Select a direct placement destination and show its preview.
    HoverAtlasPlacement(PlacementTarget),
    /// Commit the selected destination.
    CommitAtlasPlacement,
    /// Cancel placement without closing the Atlas.
    CancelAtlasPlacement,
    /// Focus a visible tile by stable activity identity.
    FocusAtlasTile(String),
    /// Focus one of the first four visible desktop tiles.
    FocusAtlasTileNumber(u8),
    /// Show contextual details without restoring or granting authority.
    ShowAtlasDetails(String),
    /// Advance deterministic preview projections.
    AdvanceLivePreviews(u64),
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

/// Backend-neutral canonical key actions for Atlas chrome.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AtlasKey {
    /// Return or Enter.
    Enter,
    /// Space.
    Space,
    /// Shift+Return or Shift+Enter.
    ShiftEnter,
    /// Shift+Space.
    ShiftSpace,
    /// Escape.
    Escape,
    /// Left or Up.
    Previous,
    /// Right or Down.
    Next,
    /// Number keys 1 through 4.
    Number(u8),
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
    /// Shell-owned Activity Atlas state.
    pub atlas: AtlasState,
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
            atlas: AtlasState::default(),
            master_percent: crate::layout::MASTER_DEFAULT_PERCENT,
            theme: ThemeConfig::default(),
        }
    }
}

impl ShellState {
    /// Apply a command and return whether visible state changed.
    pub fn apply(&mut self, command: Command) -> bool {
        self.apply_with_context(command, false, &[])
    }

    /// Apply a command with the active presentation and valid activity set.
    pub fn apply_with_context(
        &mut self,
        command: Command,
        phone: bool,
        atlas_activity_ids: &[String],
    ) -> bool {
        let before = self.clone();
        match command {
            Command::ToggleLauncher => self.launcher_open = !self.launcher_open,
            Command::ToggleAtlas(invocation) => {
                if self.atlas.open {
                    self.atlas.close();
                } else {
                    self.atlas.open(invocation);
                }
            }
            Command::ActivateAtlasTile(activity_id) => {
                if self.atlas.open
                    && atlas_activity_ids.iter().any(|id| id == &activity_id)
                    && let Ok(id) = crate::atlas::AtlasActivityId::new(activity_id)
                {
                    self.atlas.select(&id);
                    self.activity_id = id.as_str().to_owned();
                    self.atlas.close();
                }
            }
            Command::BeginAtlasPlacement {
                activity_id,
                origin,
            } => {
                if self.atlas.open
                    && atlas_activity_ids.iter().any(|id| id == &activity_id)
                    && let Ok(id) = crate::atlas::AtlasActivityId::new(activity_id)
                {
                    self.atlas.begin_placement(&id, origin, phone);
                }
            }
            Command::HoverAtlasPlacement(target) => {
                self.atlas.hover_placement(target, phone);
            }
            Command::CommitAtlasPlacement => {
                self.atlas
                    .commit_placement(phone, self.theme.reduced_motion);
            }
            Command::CancelAtlasPlacement => {
                self.atlas.cancel_placement();
            }
            Command::FocusAtlasTile(activity_id) => {
                if self.atlas.open
                    && atlas_activity_ids.iter().any(|id| id == &activity_id)
                    && let Ok(id) = crate::atlas::AtlasActivityId::new(activity_id)
                {
                    self.atlas.select(&id);
                }
            }
            Command::FocusAtlasTileNumber(number) => {
                if self.atlas.open
                    && !phone
                    && (1..=4).contains(&number)
                    && let Some(id) = atlas_activity_ids.get(usize::from(number - 1))
                    && let Ok(id) = crate::atlas::AtlasActivityId::new(id.clone())
                {
                    self.atlas.select(&id);
                }
            }
            Command::ShowAtlasDetails(activity_id) => {
                if self.atlas.open
                    && atlas_activity_ids.iter().any(|id| id == &activity_id)
                    && let Ok(id) = crate::atlas::AtlasActivityId::new(activity_id)
                {
                    self.atlas.show_details(&id);
                }
            }
            Command::AdvanceLivePreviews(milliseconds) => {
                self.atlas.preview_ms = self.atlas.preview_ms.saturating_add(milliseconds);
            }
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

    /// Apply the accepted Sol keyboard contract to visible Atlas chrome.
    pub fn apply_atlas_key(
        &mut self,
        key: AtlasKey,
        phone: bool,
        atlas_activity_ids: &[String],
    ) -> bool {
        let before = self.clone();
        let selected = self.atlas.selected_activity.clone();
        match key {
            AtlasKey::Enter | AtlasKey::Space => {
                if self.atlas.placement.is_some() {
                    self.atlas
                        .commit_placement(phone, self.theme.reduced_motion);
                } else if let (true, Some(selected)) = (self.atlas.open, selected) {
                    self.activity_id = selected.as_str().to_owned();
                    self.atlas.close();
                }
            }
            AtlasKey::ShiftEnter | AtlasKey::ShiftSpace => {
                if self.atlas.open
                    && let Some(selected) = selected
                    && atlas_activity_ids.iter().any(|id| id == selected.as_str())
                {
                    self.atlas
                        .begin_placement(&selected, PlacementOrigin::Keyboard, phone);
                }
            }
            AtlasKey::Escape => {
                if !self.atlas.cancel_placement() {
                    self.atlas.close();
                }
            }
            AtlasKey::Previous | AtlasKey::Next => {
                let forward = key == AtlasKey::Next;
                if self.atlas.placement.is_some() {
                    self.atlas.move_placement(forward, phone);
                } else if self.atlas.open {
                    let Some(current) =
                        self.atlas.selected_activity.as_ref().and_then(|selected| {
                            atlas_activity_ids
                                .iter()
                                .position(|id| id == selected.as_str())
                        })
                    else {
                        return false;
                    };
                    let count = atlas_activity_ids.len();
                    let next = if forward {
                        (current + 1) % count
                    } else {
                        (current + count - 1) % count
                    };
                    if let Some(id) = atlas_activity_ids.get(next)
                        && let Ok(id) = crate::atlas::AtlasActivityId::new(id.clone())
                    {
                        self.atlas.select(&id);
                    }
                }
            }
            AtlasKey::Number(number) => {
                self.apply_with_context(
                    Command::FocusAtlasTileNumber(number),
                    phone,
                    atlas_activity_ids,
                );
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

    #[test]
    fn pointer_command_space_and_super_space_are_one_atlas_action() {
        for invocation in [
            crate::atlas::AtlasInvocation::Pointer,
            crate::atlas::AtlasInvocation::CommandSpace,
            crate::atlas::AtlasInvocation::SuperSpace,
        ] {
            let mut state = ShellState::default();
            assert!(state.apply(Command::ToggleAtlas(invocation)));
            let opened = state.clone();
            assert!(opened.atlas.open);
            assert!(state.apply(Command::ToggleAtlas(invocation)));
            assert!(!state.atlas.open);
            assert!(opened.atlas.focus_origin.is_some());
        }
    }

    #[test]
    fn atlas_identity_is_preserved_by_restore() {
        let mut state = ShellState::default();
        let ids = vec!["make".to_owned()];
        state.apply_with_context(
            Command::ToggleAtlas(crate::atlas::AtlasInvocation::SuperSpace),
            false,
            &ids,
        );
        state.apply_with_context(Command::ActivateAtlasTile("make".to_owned()), false, &ids);
        assert_eq!(state.activity_id, "make");
        assert!(!state.atlas.open);
    }

    #[test]
    fn atlas_keyboard_matches_drag_and_escape_semantics() {
        let ids = (1..=5)
            .map(|index| format!("activity-{index}"))
            .collect::<Vec<_>>();
        let mut state = ShellState::default();
        assert!(state.apply(Command::ToggleAtlas(
            crate::atlas::AtlasInvocation::SuperSpace
        )));
        assert!(state.apply_atlas_key(AtlasKey::Number(1), false, &ids));
        assert!(state.apply_atlas_key(AtlasKey::ShiftEnter, false, &ids));
        assert!(state.apply_atlas_key(AtlasKey::Next, false, &ids));
        assert_eq!(
            state.atlas.placement.as_ref().map(|choice| choice.target),
            Some(crate::atlas::PlacementTarget::Tab)
        );
        assert!(state.apply_atlas_key(AtlasKey::Enter, false, &ids));
        assert_eq!(
            state.atlas.last_commit.as_ref().map(|commit| commit.target),
            Some(crate::atlas::PlacementTarget::Tab)
        );
        assert!(state.atlas.placement.is_none());

        assert!(state.apply_atlas_key(AtlasKey::ShiftEnter, false, &ids));
        assert!(state.apply_atlas_key(AtlasKey::Escape, false, &ids));
        assert!(state.atlas.placement.is_none());
        assert!(state.atlas.open);
        assert!(state.apply_atlas_key(AtlasKey::Escape, false, &ids));
        assert!(!state.atlas.open);
    }

    #[test]
    fn phone_shift_enter_then_enter_commits_new_activity_without_hover() {
        let ids = vec!["activity-a".to_owned()];
        let mut state = ShellState::default();
        assert!(state.apply_with_context(
            Command::ToggleAtlas(crate::atlas::AtlasInvocation::SuperSpace),
            true,
            &ids,
        ));
        assert!(state.apply_with_context(
            Command::FocusAtlasTile("activity-a".to_owned()),
            true,
            &ids,
        ));
        assert!(state.apply_atlas_key(AtlasKey::ShiftEnter, true, &ids));
        assert_eq!(
            state.atlas.placement.as_ref().map(|choice| choice.target),
            Some(crate::atlas::PlacementTarget::NewActivity)
        );
        assert!(state.apply_atlas_key(AtlasKey::Enter, true, &ids));
        assert_eq!(
            state.atlas.last_commit.as_ref().map(|commit| commit.target),
            Some(crate::atlas::PlacementTarget::NewActivity)
        );
    }

    #[test]
    fn phone_drag_begin_commits_new_activity_without_hover() {
        let ids = vec!["activity-a".to_owned()];
        let mut state = ShellState::default();
        assert!(state.apply_with_context(
            Command::ToggleAtlas(crate::atlas::AtlasInvocation::Pointer),
            true,
            &ids,
        ));
        assert!(state.apply_with_context(
            Command::BeginAtlasPlacement {
                activity_id: "activity-a".to_owned(),
                origin: PlacementOrigin::Drag,
            },
            true,
            &ids,
        ));
        assert_eq!(
            state.atlas.placement.as_ref().map(|choice| choice.target),
            Some(crate::atlas::PlacementTarget::NewActivity)
        );
        assert!(state.apply_atlas_key(AtlasKey::Enter, true, &ids));
        assert_eq!(
            state.atlas.last_commit.as_ref().map(|commit| commit.target),
            Some(crate::atlas::PlacementTarget::NewActivity)
        );
    }

    #[test]
    fn phone_desktop_hover_is_rejected_and_cannot_be_committed() {
        let ids = vec!["activity-a".to_owned()];
        let mut state = ShellState::default();
        assert!(state.apply_with_context(
            Command::ToggleAtlas(crate::atlas::AtlasInvocation::Pointer),
            true,
            &ids,
        ));
        assert!(state.apply_with_context(
            Command::BeginAtlasPlacement {
                activity_id: "activity-a".to_owned(),
                origin: PlacementOrigin::Drag,
            },
            true,
            &ids,
        ));
        assert!(!state.apply_with_context(
            Command::HoverAtlasPlacement(crate::atlas::PlacementTarget::Master),
            true,
            &ids,
        ));
        assert_eq!(
            state.atlas.placement.as_ref().map(|choice| choice.target),
            Some(crate::atlas::PlacementTarget::NewActivity)
        );
        assert!(state.apply_atlas_key(AtlasKey::Enter, true, &ids));
        assert_eq!(
            state.atlas.last_commit.as_ref().map(|commit| commit.target),
            Some(crate::atlas::PlacementTarget::NewActivity)
        );
    }
}
