//! Shell-owned Activity Atlas state, placement, and layout.
//!
//! This module is intentionally separate from [`crate::components`].  The
//! Atlas is shell chrome: it presents stable activity identities and never
//! adds a semantic catalog primitive or grants authority.

use crate::activity::{OpaqueOwnerRef, OpaquePrincipalRef};
use crate::layout::{Rect, Viewport};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Maximum UTF-8 bytes in a stable Atlas activity identity.
pub const MAX_ATLAS_ACTIVITY_ID_BYTES: usize = 256;
/// Maximum UTF-8 bytes in Atlas presentation text.
pub const MAX_ATLAS_TEXT_BYTES: usize = 512;
/// Maximum number of activities presented by one Atlas.
pub const MAX_ATLAS_ACTIVITIES: usize = 64;

/// A stable activity identity used by Atlas chrome.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct AtlasActivityId(String);

impl AtlasActivityId {
    /// Construct and bound a stable identity.
    pub fn new(value: impl Into<String>) -> Result<Self, AtlasError> {
        let value = value.into();
        if value.is_empty() || value.len() > MAX_ATLAS_ACTIVITY_ID_BYTES {
            return Err(AtlasError::InvalidActivityId);
        }
        Ok(Self(value))
    }

    /// Return the stable identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AtlasActivityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for AtlasActivityId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// A bounded live preview projection.  It has no focus or input authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PreviewKind {
    /// Video preview with deterministic media time.
    Video,
    /// Editing preview with a deterministic cursor.
    Editor,
    /// Audio preview with a deterministic waveform phase.
    Audio,
    /// Plan preview with bounded semantic edits.
    Plan,
    /// Honest unavailable native portal projection.
    NativeUnavailable,
}

impl PreviewKind {
    /// Short preview description used by the display adapter.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Video => "video playing",
            Self::Editor => "editor cursor",
            Self::Audio => "waveform playing",
            Self::Plan => "plan updating",
            Self::NativeUnavailable => "native unavailable",
        }
    }
}

/// Whether a recipe was restored, changed, or is a waiting proposal.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RecipeStatus {
    /// Fixture restore completed.
    Restored,
    /// Bounded semantic edits occurred since the last open.
    Changed,
    /// A reviewed patch proposal is waiting; it is not accepted here.
    Proposal,
}

impl RecipeStatus {
    /// Short subordinate status label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Restored => "Restored",
            Self::Changed => "Changed",
            Self::Proposal => "Proposal",
        }
    }
}

/// One activity presented by the Atlas.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AtlasEntry {
    /// Stable activity identity.
    pub activity_id: AtlasActivityId,
    /// Human-readable activity title.
    pub title: String,
    /// Stable temporal context, such as "2 hours ago".
    pub temporal_context: String,
    /// Astrid-supplied workspace owner.
    pub owner_ref: OpaqueOwnerRef,
    /// Separate acting principal; it never collapses into the owner.
    pub acting_principal: OpaquePrincipalRef,
    /// Durable recipe identity.
    pub recipe_id: String,
    /// Fixture recipe revision represented.
    pub recipe_revision: u64,
    /// Recipe status shown beneath the preview.
    pub recipe_status: RecipeStatus,
    /// Number of bounded semantic edits for [`RecipeStatus::Changed`].
    pub changed_edits: u16,
    /// Non-interactive live preview kind.
    pub preview: PreviewKind,
}

impl AtlasEntry {
    /// Construct and validate an entry.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        activity_id: impl Into<String>,
        title: impl Into<String>,
        temporal_context: impl Into<String>,
        owner_ref: OpaqueOwnerRef,
        acting_principal: OpaquePrincipalRef,
        recipe_id: impl Into<String>,
        recipe_revision: u64,
        recipe_status: RecipeStatus,
        changed_edits: u16,
        preview: PreviewKind,
    ) -> Result<Self, AtlasError> {
        if recipe_revision == 0 {
            return Err(AtlasError::InvalidRecipeRevision);
        }
        let entry = Self {
            activity_id: AtlasActivityId::new(activity_id)?,
            title: title.into(),
            temporal_context: temporal_context.into(),
            owner_ref,
            acting_principal,
            recipe_id: recipe_id.into(),
            recipe_status,
            recipe_revision,
            changed_edits,
            preview,
        };
        entry.validate()?;
        Ok(entry)
    }

    fn validate(&self) -> Result<(), AtlasError> {
        for text in [&self.title, &self.temporal_context, &self.recipe_id] {
            if text.is_empty() || text.len() > MAX_ATLAS_TEXT_BYTES {
                return Err(AtlasError::InvalidText);
            }
        }
        if self.owner_ref.id().is_empty() || self.owner_ref.id().len() > MAX_ATLAS_TEXT_BYTES {
            return Err(AtlasError::InvalidOwner);
        }
        if self.recipe_status == RecipeStatus::Changed && self.changed_edits == 0 {
            return Err(AtlasError::InvalidChangedCount);
        }
        if self.recipe_status != RecipeStatus::Changed && self.changed_edits != 0 {
            return Err(AtlasError::InvalidChangedCount);
        }
        Ok(())
    }

    /// One bounded, subordinate status line.
    pub fn status_line(&self) -> String {
        match self.recipe_status {
            RecipeStatus::Restored => {
                format!(
                    "{} · recipe revision {}",
                    self.recipe_status.label(),
                    self.recipe_revision
                )
            }
            RecipeStatus::Changed => {
                format!(
                    "{} · {} semantic edits",
                    self.recipe_status.label(),
                    self.changed_edits
                )
            }
            RecipeStatus::Proposal => {
                format!("{} · reviewed patch is waiting", self.recipe_status.label())
            }
        }
    }
}

/// Pointer control or a platform shortcut that invokes the same action.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AtlasInvocation {
    /// Activities control in shell chrome.
    Pointer,
    /// Canonical Command-Space platform shortcut.
    CommandSpace,
    /// Canonical Super-Space platform shortcut.
    SuperSpace,
}

/// How a placement interaction began.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PlacementOrigin {
    /// Pointer drag.
    Drag,
    /// Shift+Enter or Shift+Space.
    Keyboard,
}

/// Direct placement destinations exposed by the Atlas.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PlacementTarget {
    /// Dominant split region.
    Master,
    /// Secondary stack region.
    Stack,
    /// Focused frame tab group.
    Tab,
    /// Floating shell frame.
    Float,
    /// New activity-context proposal.
    NewActivity,
}

impl PlacementTarget {
    /// Every desktop target in display order.
    pub const DESKTOP: [Self; 5] = [
        Self::Master,
        Self::Stack,
        Self::Tab,
        Self::Float,
        Self::NewActivity,
    ];

    /// The only meaningful target in the one-card phone presentation.
    pub const PHONE: [Self; 1] = [Self::NewActivity];

    /// Human-readable target label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Master => "Master",
            Self::Stack => "Stack",
            Self::Tab => "Tab",
            Self::Float => "Float",
            Self::NewActivity => "New activity",
        }
    }

    /// Compile to a shell layout intent, not a catalog primitive.
    pub const fn intent(self) -> PlacementIntent {
        match self {
            Self::Master => PlacementIntent::SplitMaster,
            Self::Stack => PlacementIntent::Stack,
            Self::Tab => PlacementIntent::TabGroup,
            Self::Float => PlacementIntent::FloatingRegion,
            Self::NewActivity => PlacementIntent::NewActivityProposal,
        }
    }
}

/// Shell layout intent compiled from a placement target.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PlacementIntent {
    /// Dominant side of the existing split intent.
    SplitMaster,
    /// Existing stack intent.
    Stack,
    /// Focused frame tab group.
    TabGroup,
    /// Existing floating region intent.
    FloatingRegion,
    /// A proposal for a new activity context.
    NewActivityProposal,
}

/// Active direct placement interaction.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlacementChoice {
    /// Stable activity being placed.
    pub activity_id: AtlasActivityId,
    /// Currently hovered or keyboard-selected destination.
    pub target: PlacementTarget,
    /// Whether pointer drag or keyboard began this interaction.
    pub origin: PlacementOrigin,
}

/// Completed fixture placement and identity-continuity report.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlacementCommit {
    /// Stable activity that retained its identity.
    pub activity_id: AtlasActivityId,
    /// Destination target.
    pub target: PlacementTarget,
    /// Compiled shell intent.
    pub intent: PlacementIntent,
    /// Whether reduced motion asks for an immediate state change.
    pub reduced_motion: bool,
    /// Source Atlas tile rectangle, when captured by the runner.
    pub source_rect: Option<Rect>,
    /// Destination frame rectangle, when captured by the runner.
    pub destination_rect: Option<Rect>,
}

/// Which invoker should receive focus when the Atlas closes.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AtlasFocusOrigin {
    /// Activities control.
    ActivitiesControl,
    /// Command-Space invoker.
    CommandSpace,
    /// Super-Space invoker.
    SuperSpace,
}

/// Transient Atlas state owned by shell chrome.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AtlasState {
    /// Whether the full-screen overview is open.
    pub open: bool,
    /// Currently focused or selected stable activity.
    pub selected_activity: Option<AtlasActivityId>,
    /// Invoker that receives focus after close.
    pub focus_origin: Option<AtlasFocusOrigin>,
    /// Active placement interaction, if any.
    pub placement: Option<PlacementChoice>,
    /// Last committed fixture placement.
    pub last_commit: Option<PlacementCommit>,
    /// Activity shown by the contextual details/review action.
    pub details_activity: Option<AtlasActivityId>,
    /// Deterministic liveness offset in milliseconds.
    pub preview_ms: u64,
}

impl AtlasState {
    /// Open with the same semantic action for every invocation.
    pub fn open(&mut self, invocation: AtlasInvocation) -> bool {
        if self.open {
            return false;
        }
        self.open = true;
        self.focus_origin = Some(match invocation {
            AtlasInvocation::Pointer => AtlasFocusOrigin::ActivitiesControl,
            AtlasInvocation::CommandSpace => AtlasFocusOrigin::CommandSpace,
            AtlasInvocation::SuperSpace => AtlasFocusOrigin::SuperSpace,
        });
        self.placement = None;
        self.details_activity = None;
        true
    }

    /// Close and return the invoking focus target.
    pub fn close(&mut self) -> Option<AtlasFocusOrigin> {
        if !self.open {
            return None;
        }
        self.open = false;
        self.placement = None;
        self.details_activity = None;
        self.focus_origin.take()
    }

    /// Select a stable activity without restoring it.
    pub fn select(&mut self, activity_id: &AtlasActivityId) -> bool {
        if self.selected_activity.as_ref() != Some(activity_id) {
            self.selected_activity = Some(activity_id.clone());
            true
        } else {
            false
        }
    }

    /// Start pointer or keyboard placement for one stable identity.
    ///
    /// Phone placement has one legal destination, so both origins begin on
    /// [`PlacementTarget::NewActivity`].
    pub fn begin_placement(
        &mut self,
        activity_id: &AtlasActivityId,
        origin: PlacementOrigin,
        phone: bool,
    ) -> bool {
        if !self.open {
            return false;
        }
        let changed = self.select(activity_id);
        let target = if phone {
            PlacementTarget::NewActivity
        } else if origin == PlacementOrigin::Drag {
            PlacementTarget::Master
        } else {
            PlacementTarget::Stack
        };
        let next = Some(PlacementChoice {
            activity_id: activity_id.clone(),
            target,
            origin,
        });
        if self.placement != next {
            self.placement = next;
            true
        } else {
            changed
        }
    }

    /// Change the selected placement destination.
    pub fn hover_placement(&mut self, target: PlacementTarget, phone: bool) -> bool {
        if phone && !PlacementTarget::PHONE.contains(&target) {
            return false;
        }
        let Some(choice) = self.placement.as_mut() else {
            return false;
        };
        if choice.target == target {
            return false;
        }
        choice.target = target;
        true
    }

    /// Move among exact destinations in display order.
    pub fn move_placement(&mut self, forward: bool, phone: bool) -> bool {
        let Some(choice) = self.placement.as_mut() else {
            return false;
        };
        let targets = if phone {
            PlacementTarget::PHONE.as_slice()
        } else {
            PlacementTarget::DESKTOP.as_slice()
        };
        let count = targets.len();
        let mut index = targets
            .iter()
            .position(|candidate| *candidate == choice.target)
            .unwrap_or_default();
        index = if forward {
            (index + 1) % count
        } else {
            (index + count - 1) % count
        };
        self.hover_placement(targets[index], phone)
    }

    /// Cancel placement; Escape does not close the Atlas in this state.
    pub fn cancel_placement(&mut self) -> bool {
        self.placement.take().is_some()
    }

    /// Commit placement and retain the same stable activity identity.
    pub fn commit_placement(
        &mut self,
        phone: bool,
        reduced_motion: bool,
    ) -> Option<PlacementCommit> {
        let choice = self.placement.clone()?;
        let allowed = if phone {
            PlacementTarget::PHONE.as_slice()
        } else {
            PlacementTarget::DESKTOP.as_slice()
        };
        if !allowed.contains(&choice.target) {
            return None;
        }
        let commit = PlacementCommit {
            activity_id: choice.activity_id,
            target: choice.target,
            intent: choice.target.intent(),
            reduced_motion,
            source_rect: None,
            destination_rect: None,
        };
        self.placement = None;
        self.last_commit = Some(commit.clone());
        Some(commit)
    }

    /// Open contextual details; this does not restore the activity.
    pub fn show_details(&mut self, activity_id: &AtlasActivityId) -> bool {
        if !self.open || self.details_activity.as_ref() == Some(activity_id) {
            return false;
        }
        self.details_activity = Some(activity_id.clone());
        true
    }
}

/// A placed Atlas tile in reading order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AtlasTile {
    /// Stable activity identity.
    pub activity_id: &'static str,
    /// Bounded tile rectangle.
    pub rect: Rect,
    /// Whether this is the selected tile.
    pub selected: bool,
}

/// A direct placement destination.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlacementTile {
    /// Destination target.
    pub target: PlacementTarget,
    /// Bounded target rectangle.
    pub rect: Rect,
    /// Whether the target has a placement preview.
    pub active: bool,
}

/// Deterministic Atlas geometry for a backend-neutral display adapter.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AtlasLayout {
    /// Phone uses the one-card semantic presentation.
    pub phone: bool,
    /// Visible desktop tiles; phone always has at most one.
    pub tiles: Vec<AtlasTile>,
    /// Placement targets currently exposed.
    pub placements: Vec<PlacementTile>,
    /// Persistent phone activity strip.
    pub strip: Option<Rect>,
    /// Placement shelf, when placement is active.
    pub shelf: Option<Rect>,
}

/// Stateless responsive Atlas layout resolver.
#[derive(Clone, Copy, Debug, Default)]
pub struct AtlasLayoutPolicy;

impl AtlasLayoutPolicy {
    /// Resolve full-screen Atlas geometry.
    pub fn resolve(
        viewport: Viewport,
        count: usize,
        selected_index: Option<usize>,
        placement: Option<(PlacementTarget, bool)>,
    ) -> AtlasLayout {
        let phone = viewport.is_phone();
        let mut layout = AtlasLayout {
            phone,
            ..AtlasLayout::default()
        };
        let targets = if phone {
            PlacementTarget::PHONE.as_slice()
        } else {
            PlacementTarget::DESKTOP.as_slice()
        };
        let shelf_height = if placement.is_some() {
            if phone { 92 } else { 124 }
        } else {
            0
        };
        let strip_height = if phone { 68 } else { 0 };
        let bottom = shelf_height + strip_height;
        let top = if phone { 60 } else { 72 };
        let content_y = top + 12;
        let content_height = viewport.height.saturating_sub(content_y + bottom + 16);
        if phone {
            if count > 0 {
                layout.tiles.push(AtlasTile {
                    activity_id: "phone-overview-card",
                    rect: Rect {
                        x: 12,
                        y: content_y,
                        width: viewport.width.saturating_sub(24),
                        height: content_height.max(220),
                    },
                    selected: true,
                });
            }
            layout.strip = Some(Rect {
                x: 0,
                y: viewport.height.saturating_sub(strip_height),
                width: viewport.width,
                height: strip_height,
            });
        } else {
            let gap = 14;
            let columns = 2;
            let available_width = viewport
                .width
                .saturating_sub(48 + gap * (columns - 1) as u32);
            let tile_width = available_width / columns as u32;
            let rows = count.div_ceil(columns);
            let tile_height = if rows == 0 {
                0
            } else {
                content_height.saturating_sub(gap * (rows.saturating_sub(1)) as u32) / rows as u32
            };
            for index in 0..count.min(MAX_ATLAS_ACTIVITIES) {
                let column = index % columns;
                let row = index / columns;
                layout.tiles.push(AtlasTile {
                    activity_id: "desktop-overview-tile",
                    rect: Rect {
                        x: 24 + column as u32 * (tile_width + gap),
                        y: content_y + row as u32 * (tile_height + gap),
                        width: tile_width,
                        height: tile_height,
                    },
                    selected: selected_index == Some(index),
                });
            }
        }
        if let Some((target, active)) = placement {
            let shelf_y = viewport.height.saturating_sub(bottom + 12);
            let shelf_rect = Rect {
                x: if phone { 8 } else { 24 },
                y: shelf_y,
                width: viewport.width.saturating_sub(if phone { 16 } else { 48 }),
                height: shelf_height,
            };
            let gap = 12;
            let tile_width = shelf_rect
                .width
                .saturating_sub(gap * (targets.len().saturating_sub(1)) as u32)
                / targets.len() as u32;
            let tile_height = shelf_rect.height.saturating_sub(38);
            for (index, candidate) in targets.iter().enumerate() {
                layout.placements.push(PlacementTile {
                    target: *candidate,
                    rect: Rect {
                        x: shelf_rect.x + index as u32 * (tile_width + gap),
                        y: shelf_rect.y + 30,
                        width: tile_width,
                        height: tile_height,
                    },
                    active: active && *candidate == target,
                });
            }
            layout.shelf = Some(shelf_rect);
        }
        layout
    }
}

/// Atlas validation failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AtlasError {
    /// Stable activity identity was empty or oversized.
    InvalidActivityId,
    /// Presentation text was empty or oversized.
    InvalidText,
    /// Owner reference was empty or oversized.
    InvalidOwner,
    /// Recipe revision was zero.
    InvalidRecipeRevision,
    /// Changed edit count did not match the recipe status.
    InvalidChangedCount,
    /// Maximum Atlas entry count was exceeded.
    TooManyActivities,
    /// The same stable identity occurred twice.
    DuplicateActivity,
}

impl fmt::Display for AtlasError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidActivityId => f.write_str("invalid Atlas activity identity"),
            Self::InvalidText => f.write_str("invalid Atlas presentation text"),
            Self::InvalidOwner => f.write_str("invalid Atlas owner reference"),
            Self::InvalidRecipeRevision => f.write_str("Atlas recipe revision cannot be zero"),
            Self::InvalidChangedCount => f.write_str("changed edit count does not match status"),
            Self::TooManyActivities => f.write_str("Atlas activity count exceeded"),
            Self::DuplicateActivity => f.write_str("duplicate Atlas activity identity"),
        }
    }
}

impl std::error::Error for AtlasError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, status: RecipeStatus, edits: u16, preview: PreviewKind) -> AtlasEntry {
        AtlasEntry::new(
            id,
            "Title",
            "Today",
            OpaqueOwnerRef::User("owner".to_owned()),
            OpaquePrincipalRef::Agent("agent".to_owned()),
            "recipe",
            1,
            status,
            edits,
            preview,
        )
        .expect("valid entry")
    }

    #[test]
    fn entry_requires_owner_principal_and_subordinate_status() {
        let restored = entry("activity-a", RecipeStatus::Restored, 0, PreviewKind::Video);
        assert!(restored.owner_ref.id() != "agent");
        assert_eq!(restored.status_line(), "Restored · recipe revision 1");
        let changed = entry("activity-b", RecipeStatus::Changed, 4, PreviewKind::Editor);
        assert_eq!(changed.status_line(), "Changed · 4 semantic edits");
        let proposal = entry("activity-c", RecipeStatus::Proposal, 0, PreviewKind::Plan);
        assert_eq!(
            proposal.status_line(),
            "Proposal · reviewed patch is waiting"
        );
    }

    #[test]
    fn placement_targets_compile_to_shell_intent() {
        assert_eq!(
            PlacementTarget::Master.intent(),
            PlacementIntent::SplitMaster
        );
        assert_eq!(PlacementTarget::Stack.intent(), PlacementIntent::Stack);
        assert_eq!(PlacementTarget::Tab.intent(), PlacementIntent::TabGroup);
        assert_eq!(
            PlacementTarget::Float.intent(),
            PlacementIntent::FloatingRegion
        );
        assert_eq!(
            PlacementTarget::NewActivity.intent(),
            PlacementIntent::NewActivityProposal
        );
    }

    #[test]
    fn identity_survives_placement_commit() {
        let mut state = AtlasState::default();
        let id = AtlasActivityId::new("activity-a").expect("identity");
        assert!(state.open(AtlasInvocation::SuperSpace));
        assert!(state.begin_placement(&id, PlacementOrigin::Keyboard, false));
        assert!(state.commit_placement(false, true).is_some());
        assert_eq!(state.last_commit.as_ref().unwrap().activity_id, id);
    }

    #[test]
    fn phone_commit_only_accepts_new_activity() {
        let mut state = AtlasState::default();
        let id = AtlasActivityId::new("activity-a").expect("identity");
        state.open(AtlasInvocation::Pointer);
        state.begin_placement(&id, PlacementOrigin::Drag, false);
        state.hover_placement(PlacementTarget::Master, false);
        assert!(state.commit_placement(true, false).is_none());
        state.hover_placement(PlacementTarget::NewActivity, false);
        assert_eq!(
            state.commit_placement(true, false).unwrap().target,
            PlacementTarget::NewActivity
        );
    }

    #[test]
    fn phone_begin_defaults_keyboard_and_drag_to_new_activity() {
        for origin in [PlacementOrigin::Keyboard, PlacementOrigin::Drag] {
            let mut state = AtlasState::default();
            let id = AtlasActivityId::new("activity-a").expect("identity");
            state.open(AtlasInvocation::Pointer);
            assert!(state.begin_placement(&id, origin, true));
            assert_eq!(
                state.placement.as_ref().map(|choice| choice.target),
                Some(PlacementTarget::NewActivity)
            );
            assert_eq!(
                state.commit_placement(true, false).unwrap().target,
                PlacementTarget::NewActivity
            );
        }
    }

    #[test]
    fn phone_hover_rejects_desktop_targets() {
        let mut state = AtlasState::default();
        let id = AtlasActivityId::new("activity-a").expect("identity");
        state.open(AtlasInvocation::Pointer);
        assert!(state.begin_placement(&id, PlacementOrigin::Drag, true));
        for target in [
            PlacementTarget::Master,
            PlacementTarget::Stack,
            PlacementTarget::Tab,
            PlacementTarget::Float,
        ] {
            assert!(!state.hover_placement(target, true));
            assert_eq!(
                state.placement.as_ref().map(|choice| choice.target),
                Some(PlacementTarget::NewActivity)
            );
        }
        assert_eq!(
            state.commit_placement(true, false).unwrap().target,
            PlacementTarget::NewActivity
        );
    }

    #[test]
    fn layout_limits_phone_to_one_card_and_one_target() {
        let mut layout = AtlasLayoutPolicy::resolve(
            Viewport::new(390, 844),
            5,
            Some(0),
            Some((PlacementTarget::NewActivity, true)),
        );
        assert_eq!(layout.tiles.len(), 1);
        assert_eq!(layout.placements.len(), 1);
        assert!(layout.strip.is_some());
        assert_eq!(
            layout.strip.map(|strip| (strip.y, strip.height)),
            Some((776, 68))
        );
        layout = AtlasLayoutPolicy::resolve(
            Viewport::new(1440, 1000),
            5,
            Some(1),
            Some((PlacementTarget::Float, true)),
        );
        assert_eq!(layout.tiles.len(), 5);
        assert_eq!(layout.placements.len(), 5);
        let placement_layout = AtlasLayoutPolicy::resolve(
            Viewport::new(390, 844),
            5,
            Some(0),
            Some((PlacementTarget::NewActivity, true)),
        );
        assert_eq!(
            placement_layout.strip.map(|strip| (strip.y, strip.height)),
            Some((776, 68))
        );
        assert_eq!(
            placement_layout.shelf.map(|shelf| shelf.y),
            Some(776 - 92 - 12)
        );
    }

    #[test]
    fn duplicate_typed_identity_is_rejected() {
        let first = AtlasActivityId::new("same").expect("identity");
        let mut entries = std::collections::BTreeMap::new();
        entries.insert(
            first.clone(),
            entry("same", RecipeStatus::Restored, 0, PreviewKind::Video),
        );
        assert!(
            entries
                .insert(
                    first.clone(),
                    entry("same", RecipeStatus::Changed, 1, PreviewKind::Editor)
                )
                .is_some()
        );
    }
}
