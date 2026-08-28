//! Deterministic activity fixtures and headless snapshot generation.

use crate::activity::{
    Activity, OpaqueOwnerRef, Patch, PatchOutcome, Recipe, RecipeStore, SurfaceId,
};
use crate::components::NodeId;
use crate::components::{ComponentKind, PropValue, SemanticNode, StateSet};
use crate::input::{Command, ShellState};
use crate::layout::{LayoutPlan, LayoutPolicy, Rect, SlotKind, Viewport};
use crate::reconcile::Reconciler;
use crate::render::{Color, DisplayList, DisplaySummary, DrawCommand};
use crate::theme::{Density, Theme, ThemeConfig, ThemeName, TokenValue};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Deterministic fixture choices accepted by the binary.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FixtureKind {
    /// Cat-break activity with video, code, and music surfaces.
    Desktop,
    /// The same activity under the phone one-surface policy.
    Phone,
    /// Complete semantic primitive and state inventory.
    ThemeLab,
}

impl FixtureKind {
    /// Parse a CLI fixture name.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "desktop" => Some(Self::Desktop),
            "phone" => Some(Self::Phone),
            "theme-lab" | "theme_lab" => Some(Self::ThemeLab),
            _ => None,
        }
    }

    /// Stable display name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::Phone => "phone",
            Self::ThemeLab => "theme-lab",
        }
    }
}

impl fmt::Display for FixtureKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Honest state of an external native portal fixture.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NativePortalState {
    /// No host backend is attached.
    Unavailable,
    /// A host backend would be starting.
    Launching,
    /// A host backend reports a surface.
    Ready,
    /// Surface has input focus.
    Focused,
    /// Host backend is stopping.
    Stopping,
    /// Host backend failed.
    Failed,
}

/// A deterministic monotonic clock used by replayable fixtures.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeterministicClock {
    /// Current logical milliseconds.
    pub now_ms: u64,
}

impl DeterministicClock {
    /// Construct a clock at a fixed instant.
    pub const fn new(now_ms: u64) -> Self {
        Self { now_ms }
    }

    /// Read the current instant.
    pub const fn now(self) -> u64 {
        self.now_ms
    }

    /// Advance by a deterministic amount.
    pub fn advance(&mut self, milliseconds: u64) {
        self.now_ms = self.now_ms.saturating_add(milliseconds);
    }
}

/// A runnable fixture containing the semantic graph and shell state.
pub struct Fixture {
    /// Fixture kind.
    pub kind: FixtureKind,
    /// Fixed logical viewport.
    pub viewport: Viewport,
    /// Current shell state.
    pub state: ShellState,
    /// Durable activity identity.
    pub activity: Activity,
    /// Durable recipe intent.
    pub recipe: Recipe,
    /// Ephemeral materialized surface.
    pub surface: crate::activity::Surface,
    /// Resolved Fieldglass theme.
    pub theme: Theme,
    /// Deterministic fixture clock.
    pub clock: DeterministicClock,
    /// Keyed retained graph.
    pub reconciler: Reconciler,
    recipes: RecipeStore,
}

impl Fixture {
    /// Build a fixture using the normative default viewport and theme.
    pub fn new(kind: FixtureKind, config: ThemeConfig) -> Result<Self, FixtureError> {
        let viewport = match kind {
            FixtureKind::Desktop | FixtureKind::ThemeLab => Viewport::new(1440, 1000),
            FixtureKind::Phone => Viewport::new(390, 844),
        };
        let root = match kind {
            FixtureKind::ThemeLab => theme_lab_root(),
            FixtureKind::Desktop | FixtureKind::Phone => activity_root(),
        };
        let recipe = Recipe::new(
            OpaqueOwnerRef::Principal("fixture-principal".to_owned()),
            if kind == FixtureKind::ThemeLab {
                "theme-lab"
            } else {
                "cat-break"
            },
            "fieldglass",
            root,
        )
        .map_err(FixtureError::Recipe)?;
        let activity = Activity::new(
            if kind == FixtureKind::ThemeLab {
                "theme-lab"
            } else {
                "cats"
            },
            OpaqueOwnerRef::Principal("fixture-principal".to_owned()),
            if kind == FixtureKind::ThemeLab {
                "Theme Lab"
            } else {
                "Cat break"
            },
            recipe.recipe_id.clone(),
        );
        let surface_id = SurfaceId::new("surface-1").map_err(FixtureError::Scene)?;
        let surface = recipe.surface(surface_id, 1);
        let theme = Theme::resolve(config);
        let mut reconciler = Reconciler::new();
        reconciler
            .reconcile(&surface.root)
            .map_err(FixtureError::Scene)?;
        let mut recipes = RecipeStore::new();
        recipes
            .insert(recipe.clone())
            .map_err(FixtureError::Patch)?;
        Ok(Self {
            kind,
            viewport,
            state: ShellState {
                activity_id: activity.activity_id.clone(),
                theme: config,
                ..ShellState::default()
            },
            activity,
            recipe,
            surface,
            theme,
            clock: DeterministicClock::new(1_724_847_360_000),
            reconciler,
            recipes,
        })
    }

    /// Apply a command and rematerialize the surface if its semantic state changed.
    pub fn apply(&mut self, command: Command) -> Result<bool, FixtureError> {
        let changed = self.state.apply(command);
        self.theme = Theme::resolve(self.state.theme);
        if changed {
            self.reconciler
                .reconcile(&self.surface.root)
                .map_err(FixtureError::Scene)?;
        }
        Ok(changed)
    }

    /// Apply a reviewed recipe patch, then rematerialize the ephemeral surface.
    pub fn apply_patch(&mut self, patch: &Patch) -> Result<PatchOutcome, FixtureError> {
        let outcome = self
            .recipes
            .apply_patch(patch)
            .map_err(FixtureError::Patch)?;
        let (recipe, visual_changed) = match &outcome {
            PatchOutcome::Applied {
                recipe,
                visual_changed,
            } => (recipe, *visual_changed),
            PatchOutcome::AlreadyApplied(recipe) => (recipe, false),
        };
        self.recipe = recipe.clone();
        if visual_changed {
            self.surface = self.recipe.surface(
                self.surface.surface_id.clone(),
                self.surface
                    .incarnation
                    .checked_add(1)
                    .ok_or(FixtureError::IncarnationOverflow)?,
            );
        }
        self.activity.current_surface = Some(self.surface.surface_id.clone());
        self.reconciler
            .reconcile(&self.surface.root)
            .map_err(FixtureError::Scene)?;
        Ok(outcome)
    }

    /// Resolve the layout plan for the current shell state.
    pub fn layout(&self) -> LayoutPlan {
        let surfaces = if self.kind == FixtureKind::ThemeLab {
            1
        } else {
            3
        };
        let focused_here = self
            .state
            .focus
            .as_ref()
            .is_some_and(|focus| focus.surface_id == self.surface.surface_id);
        LayoutPolicy::resolve(
            self.viewport,
            if focused_here {
                crate::layout::LayoutMode::Focus
            } else {
                self.state.layout
            },
            self.state.master_percent,
            surfaces,
        )
    }

    /// Produce a backend-neutral display list.
    pub fn display_list(&self) -> DisplayList {
        if self.kind == FixtureKind::ThemeLab {
            theme_lab_display(self.viewport, &self.theme, &self.surface.root)
        } else {
            activity_display(
                self.viewport,
                &self.theme,
                &self.state,
                &self.layout(),
                &self.surface.root,
            )
        }
    }

    /// Produce deterministic semantic and display summaries.
    pub fn snapshot(&self) -> Snapshot {
        let display = self.display_list();
        let semantic_bytes =
            serde_json::to_vec(&(&self.activity, &self.recipe, &self.surface, &self.state))
                .expect("fixture is serializable");
        let semantic_digest = blake3::hash(&semantic_bytes).to_hex().to_string();
        Snapshot {
            fixture: self.kind,
            viewport: self.viewport,
            theme: self.theme.config.name,
            density: self.theme.config.density,
            scale_percent: self.theme.config.scale.percent(),
            reduced_motion: self.theme.config.reduced_motion,
            activity_id: self.activity.activity_id.clone(),
            recipe_revision: self.recipe.revision,
            semantic_digest,
            display: display.summary(),
            surface_count: if self.kind == FixtureKind::ThemeLab {
                1
            } else {
                3
            },
            visible_surface_count: self.layout().slots.len(),
            native_portal: NativePortalState::Unavailable,
            clock_ms: self.clock.now(),
        }
    }

    /// Return the recipe store used by the fixture.
    pub fn recipe_store(&self) -> &RecipeStore {
        &self.recipes
    }
}

/// Stable output of one fixture frame.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    /// Fixture name.
    pub fixture: FixtureKind,
    /// Logical viewport.
    pub viewport: Viewport,
    /// Palette.
    pub theme: ThemeName,
    /// Density.
    pub density: Density,
    /// Text scale percentage.
    pub scale_percent: u16,
    /// Reduced-motion setting.
    pub reduced_motion: bool,
    /// Activity identity.
    pub activity_id: String,
    /// Recipe revision represented.
    pub recipe_revision: u64,
    /// Digest of semantic activity/surface state.
    pub semantic_digest: String,
    /// Display command summary.
    pub display: DisplaySummary,
    /// Number of semantic surfaces in the activity.
    pub surface_count: usize,
    /// Number of visible layout slots.
    pub visible_surface_count: usize,
    /// Honest native portal state.
    pub native_portal: NativePortalState,
    /// Fixed logical timestamp.
    pub clock_ms: u64,
}

/// Fixture construction or update failure.
#[derive(Debug)]
pub enum FixtureError {
    /// Semantic scene validation failed.
    Scene(crate::components::SceneError),
    /// Recipe operation failed.
    Patch(crate::activity::PatchError),
    /// Recipe construction or invariant validation failed.
    Recipe(crate::activity::RecipeValidationError),
    /// Surface incarnation exhausted u64.
    IncarnationOverflow,
}

impl fmt::Display for FixtureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scene(error) => error.fmt(f),
            Self::Patch(error) => error.fmt(f),
            Self::Recipe(error) => error.fmt(f),
            Self::IncarnationOverflow => write!(f, "surface incarnation overflow"),
        }
    }
}

impl std::error::Error for FixtureError {}

fn prop(node: &mut SemanticNode, key: &str, value: PropValue) {
    node.props = node
        .props
        .clone()
        .with(key.to_owned(), value)
        .expect("fixture property bounds");
}

fn child(parent: &mut SemanticNode, node: SemanticNode) {
    parent.push(node).expect("fixture child bound");
}

fn activity_root() -> SemanticNode {
    let mut root = SemanticNode::new(
        "activity/cat-break/root",
        ComponentKind::Region,
        "Cat break",
    );
    prop(&mut root, "title", PropValue::Text("Cat break".to_owned()));

    let mut cat_surface = SemanticNode::new(
        "activity/cat-break/cat-surface",
        ComponentKind::Card,
        "Cat video feed",
    );
    prop(
        &mut cat_surface,
        "subtitle",
        PropValue::Text("One feed, tuned for you".to_owned()),
    );
    let mut video = SemanticNode::new(
        "activity/cat-break/cat-surface/video",
        ComponentKind::VideoPlayer,
        "Miso discovers the warm laundry",
    );
    video.state = StateSet::empty().with(StateSet::LOADING, true);
    prop(
        &mut video,
        "title",
        PropValue::Text("Miso discovers the warm laundry".to_owned()),
    );
    prop(&mut video, "duration", PropValue::Text("2:14".to_owned()));
    prop(
        &mut video,
        "source",
        PropValue::Text("fixture://miso".to_owned()),
    );
    child(&mut cat_surface, video);
    let mut feed = SemanticNode::new(
        "activity/cat-break/cat-surface/feed",
        ComponentKind::Repeater,
        "Cat feed",
    );
    for (key, title) in [
        ("yt", "The box was occupied"),
        ("peer", "Tiny apartment lion"),
        ("vid", "Rainy-window cat TV"),
    ] {
        let mut item = SemanticNode::new(
            &format!("activity/cat-break/feed/{key}"),
            ComponentKind::MediaEmbed,
            title,
        );
        prop(&mut item, "source", PropValue::Text(key.to_owned()));
        child(&mut feed, item);
    }
    child(&mut cat_surface, feed);
    child(&mut root, cat_surface);

    let mut code = SemanticNode::new(
        "activity/cat-break/code",
        ComponentKind::CodeBlock,
        "surface-model.rs",
    );
    prop(&mut code, "language", PropValue::Text("rust".to_owned()));
    prop(
        &mut code,
        "text",
        PropValue::Text("let activity = open_activity(\"cat-break\");".to_owned()),
    );
    child(&mut root, code);

    let mut sound = SemanticNode::new(
        "activity/cat-break/sound",
        ComponentKind::AudioPlayer,
        "Light through the blinds",
    );
    sound.state = StateSet::empty().with(StateSet::SUCCESS, true);
    prop(
        &mut sound,
        "source",
        PropValue::Text("fixture://light".to_owned()),
    );
    prop(&mut sound, "duration", PropValue::Text("3:04".to_owned()));
    child(&mut root, sound);

    let mut portal = SemanticNode::new(
        "activity/cat-break/native/steam",
        ComponentKind::NativePortal,
        "Native application",
    );
    portal.state = StateSet::empty().with(StateSet::EMPTY, true);
    prop(
        &mut portal,
        "status",
        PropValue::Text("Unavailable in fixture".to_owned()),
    );
    child(&mut root, portal);
    root
}

fn theme_lab_root() -> SemanticNode {
    let mut root = SemanticNode::new("theme-lab/root", ComponentKind::Region, "Theme Lab");
    let mut grid = SemanticNode::new("theme-lab/catalog", ComponentKind::Grid, "Semantic catalog");
    for kind in ComponentKind::ALL {
        let mut sample =
            SemanticNode::new(&format!("theme-lab/{kind:?}"), kind, &format!("{kind:?}"));
        for key in kind.contract().required_props {
            let value = if matches!(*key, "min" | "max" | "step" | "columns") {
                PropValue::Number(1.0)
            } else {
                PropValue::Text(format!("{key} sample"))
            };
            prop(&mut sample, key, value);
        }
        sample.state = match kind {
            ComponentKind::Button | ComponentKind::TextField | ComponentKind::Select => {
                StateSet::empty().with(StateSet::FOCUS, true)
            }
            ComponentKind::NativePortal => StateSet::empty().with(StateSet::EMPTY, true),
            _ => StateSet::empty(),
        };
        child(&mut grid, sample);
    }
    child(&mut root, grid);
    root
}

fn semantic_display(
    viewport: Viewport,
    theme: &Theme,
    state: &ShellState,
    root: &SemanticNode,
    list: &mut DisplayList,
) {
    let text = token_color(theme, "aos.color.text", [244, 242, 247, 255]);
    let focus = token_color(theme, "aos.color.focus", [111, 180, 255, 255]);
    let rect = Rect {
        x: 12,
        y: 54,
        width: viewport.width.saturating_sub(24),
        height: viewport.height.saturating_sub(66),
    };
    let focus_id = state.focus.as_ref().and_then(|target| target.node_id);
    render_semantic_node(list, root, theme, rect, focus_id, 0, text, focus);
}

#[allow(clippy::too_many_arguments)]
fn render_semantic_node(
    list: &mut DisplayList,
    node: &SemanticNode,
    theme: &Theme,
    rect: Rect,
    focus: Option<NodeId>,
    depth: usize,
    text: Color,
    focus_color: Color,
) {
    if Some(node.id) == focus {
        list.push(DrawCommand::StrokeRoundRect {
            rect,
            radius: 3,
            color: focus_color,
            width: 3,
        });
    }
    if node.state.bits() != 0 {
        list.push(DrawCommand::StrokeRoundRect {
            rect,
            radius: 4,
            color: token_color(theme, "aos.color.accent", [157, 122, 255, 255]),
            width: 2,
        });
    }
    if depth > 0 {
        list.push(DrawCommand::StrokeRoundRect {
            rect,
            radius: 3,
            color: token_color(theme, "aos.color.line", [61, 62, 71, 255]),
            width: 1,
        });
    }
    list.push(DrawCommand::Text {
        rect: Rect {
            x: rect.x + 8,
            y: rect.y + 4,
            width: rect.width.saturating_sub(16),
            height: 18.min(rect.height),
        },
        content: node.accessibility.name.clone(),
        role: "control".to_owned(),
        color: text,
    });
    if node.children.is_empty() || rect.height < 36 {
        return;
    }
    let gap = u32::from(token_px(theme, "aos.space.2", 7));
    let available = rect
        .height
        .saturating_sub(22 + gap * (node.children.len() as u32 - 1));
    let child_height = available / node.children.len() as u32;
    if child_height < 18 {
        return;
    }
    let mut y = rect.y + 22;
    for child in &node.children {
        render_semantic_node(
            list,
            child,
            theme,
            Rect {
                x: rect.x + 8,
                y,
                width: rect.width.saturating_sub(16),
                height: child_height,
            },
            focus,
            depth + 1,
            text,
            focus_color,
        );
        y = y.saturating_add(child_height + gap);
    }
}

fn token_color(theme: &Theme, role: &str, fallback: Color) -> Color {
    match theme.tokens.get(role) {
        Some(TokenValue::Color(color)) => *color,
        _ => fallback,
    }
}

fn token_px(theme: &Theme, role: &str, fallback: u16) -> u16 {
    match theme.tokens.get(role) {
        Some(TokenValue::Pixels(px)) => *px,
        _ => fallback,
    }
}

fn activity_display(
    viewport: Viewport,
    theme: &Theme,
    state: &ShellState,
    plan: &LayoutPlan,
    root: &SemanticNode,
) -> DisplayList {
    let canvas = token_color(theme, "aos.color.canvas", [12, 14, 17, 255]);
    let raised = token_color(theme, "aos.color.canvas-raised", [23, 25, 30, 255]);
    let layer = token_color(theme, "aos.color.layer.1", [31, 33, 40, 255]);
    let text = token_color(theme, "aos.color.text", [244, 242, 247, 255]);
    let soft = token_color(theme, "aos.color.text-soft", [194, 193, 204, 255]);
    let dim = token_color(theme, "aos.color.text-dim", [131, 130, 143, 255]);
    let line = token_color(theme, "aos.color.line", [61, 62, 71, 255]);
    let accent = token_color(theme, "aos.color.accent", [157, 122, 255, 255]);
    let focus = token_color(theme, "aos.color.focus", [111, 180, 255, 255]);
    let radius_window = token_px(theme, "aos.radius.window", 15);
    let radius_control = token_px(theme, "aos.radius.control", 9);
    let mut list = DisplayList::new((viewport.width, viewport.height));
    list.push(DrawCommand::FillRoundRect {
        rect: Rect {
            x: 0,
            y: 0,
            width: viewport.width,
            height: viewport.height,
        },
        radius: 0,
        color: canvas,
    });
    list.push(DrawCommand::FillRoundRect {
        rect: Rect {
            x: 0,
            y: 0,
            width: viewport.width,
            height: if plan.phone { 48 } else { 44 },
        },
        radius: 0,
        color: raised,
    });
    list.push(DrawCommand::Icon {
        rect: Rect {
            x: 12,
            y: 12,
            width: 20,
            height: 20,
        },
        name: "astrid-mark".to_owned(),
        color: accent,
    });
    list.push(DrawCommand::Text {
        rect: Rect {
            x: 40,
            y: 10,
            width: 130,
            height: 24,
        },
        content: "Astrid".to_owned(),
        role: "control".to_owned(),
        color: text,
    });
    list.push(DrawCommand::Text {
        rect: Rect {
            x: viewport.width / 2 - 110,
            y: 10,
            width: 220,
            height: 24,
        },
        content: "Fri, Aug 28 · 09:16 AM".to_owned(),
        role: "control".to_owned(),
        color: soft,
    });
    for slot in &plan.slots {
        frame(&mut list, slot.rect, radius_window, layer, line);
        let title = match slot.kind {
            SlotKind::Master => "Cat break",
            SlotKind::Secondary if slot.surface_index == 1 => "Build",
            SlotKind::Secondary => "Sound",
            SlotKind::Phone => "Cat break",
        };
        list.push(DrawCommand::Text {
            rect: Rect {
                x: slot.rect.x + 14,
                y: slot.rect.y + 12,
                width: slot.rect.width.saturating_sub(28),
                height: 22,
            },
            content: title.to_owned(),
            role: "title".to_owned(),
            color: text,
        });
        if slot.kind == SlotKind::Master || slot.kind == SlotKind::Phone {
            let body = Rect {
                x: slot.rect.x + 1,
                y: slot.rect.y + 42,
                width: slot.rect.width.saturating_sub(2),
                height: slot.rect.height.saturating_sub(43),
            };
            list.push(DrawCommand::FillRoundRect {
                rect: body,
                radius: radius_control,
                color: [53, 54, 78, 255],
            });
            list.push(DrawCommand::Icon {
                rect: Rect {
                    x: body.x + body.width / 2 - 84,
                    y: body.y + body.height / 2 - 102,
                    width: 168,
                    height: 168,
                },
                name: "cat-miso".to_owned(),
                color: [224, 191, 165, 255],
            });
            list.push(DrawCommand::Text {
                rect: Rect {
                    x: body.x + 14,
                    y: body.y + body.height.saturating_sub(74),
                    width: body.width.saturating_sub(28),
                    height: 28,
                },
                content: "Miso discovers the warm laundry".to_owned(),
                role: "display".to_owned(),
                color: text,
            });
            list.push(DrawCommand::Line {
                from: (body.x + 16, body.y + body.height.saturating_sub(32)),
                to: (
                    body.x + body.width.saturating_sub(16),
                    body.y + body.height.saturating_sub(32),
                ),
                color: focus,
                width: 3,
            });
        } else if slot.surface_index == 1 {
            list.push(DrawCommand::Text {
                rect: Rect {
                    x: slot.rect.x + 18,
                    y: slot.rect.y + 62,
                    width: slot.rect.width.saturating_sub(36),
                    height: 150,
                },
                content: "let activity = open_activity(\"cat-break\");\n\nactivity.propose(Patch::Refresh);".to_owned(),
                role: "body".to_owned(),
                color: soft,
            });
        } else {
            list.push(DrawCommand::Text {
                rect: Rect {
                    x: slot.rect.x + 20,
                    y: slot.rect.y + slot.rect.height / 2,
                    width: slot.rect.width.saturating_sub(40),
                    height: 30,
                },
                content: "Light through the blinds".to_owned(),
                role: "title".to_owned(),
                color: text,
            });
            list.push(DrawCommand::Line {
                from: (slot.rect.x + 20, slot.rect.y + slot.rect.height / 2 + 40),
                to: (
                    slot.rect.x + slot.rect.width.saturating_sub(20),
                    slot.rect.y + slot.rect.height / 2 + 40,
                ),
                color: accent,
                width: 3,
            });
        }
    }
    if plan.phone {
        list.push(DrawCommand::FillRoundRect {
            rect: Rect {
                x: 0,
                y: viewport.height.saturating_sub(66),
                width: viewport.width,
                height: 66,
            },
            radius: 0,
            color: raised,
        });
        for (index, label) in ["Pause", "Make", "Build", "Play", "Go"]
            .into_iter()
            .enumerate()
        {
            let width = viewport.width / 5;
            list.push(DrawCommand::Text {
                rect: Rect {
                    x: index as u32 * width,
                    y: viewport.height.saturating_sub(48),
                    width,
                    height: 28,
                },
                content: label.to_owned(),
                role: "caption".to_owned(),
                color: if index == 0 { text } else { dim },
            });
        }
    }
    if state.launcher_open {
        list.push(DrawCommand::StrokeRoundRect {
            rect: Rect {
                x: viewport.width / 2 - 340,
                y: viewport.height / 2 - 180,
                width: 680,
                height: 360,
            },
            radius: radius_window,
            color: focus,
            width: 2,
        });
    }
    // DisplayList commands are painter-ordered: retain the semantic surface
    // above every opaque canvas and chrome fill so its root remains visible.
    semantic_display(viewport, theme, state, root, &mut list);
    list
}

fn theme_lab_display(viewport: Viewport, theme: &Theme, root: &SemanticNode) -> DisplayList {
    let canvas = token_color(theme, "aos.color.canvas", [12, 14, 17, 255]);
    let raised = token_color(theme, "aos.color.canvas-raised", [23, 25, 30, 255]);
    let layer = token_color(theme, "aos.color.layer.1", [31, 33, 40, 255]);
    let text = token_color(theme, "aos.color.text", [244, 242, 247, 255]);
    let soft = token_color(theme, "aos.color.text-soft", [194, 193, 204, 255]);
    let line = token_color(theme, "aos.color.line", [61, 62, 71, 255]);
    let accent = token_color(theme, "aos.color.accent", [157, 122, 255, 255]);
    let mut list = DisplayList::new((viewport.width, viewport.height));
    list.push(DrawCommand::FillRoundRect {
        rect: Rect {
            x: 0,
            y: 0,
            width: viewport.width,
            height: viewport.height,
        },
        radius: 0,
        color: canvas,
    });
    list.push(DrawCommand::FillRoundRect {
        rect: Rect {
            x: 0,
            y: 0,
            width: viewport.width,
            height: 54,
        },
        radius: 0,
        color: raised,
    });
    list.push(DrawCommand::Text {
        rect: Rect {
            x: 54,
            y: 11,
            width: 300,
            height: 30,
        },
        content: "Theme Lab".to_owned(),
        role: "title".to_owned(),
        color: text,
    });
    list.push(DrawCommand::Text {
        rect: Rect {
            x: 54,
            y: 34,
            width: 520,
            height: 18,
        },
        content: "The complete semantic component contract · fixtures only".to_owned(),
        role: "caption".to_owned(),
        color: soft,
    });
    let sidebar_width = 220;
    list.push(DrawCommand::FillRoundRect {
        rect: Rect {
            x: 0,
            y: 54,
            width: sidebar_width,
            height: viewport.height.saturating_sub(54),
        },
        radius: 0,
        color: raised,
    });
    for (index, family) in [
        "Structure",
        "Content",
        "Input",
        "Data",
        "Navigation",
        "Feedback",
        "Governed",
        "Media",
        "Canvas",
        "Terminal",
        "Native",
        "Tokens",
    ]
    .into_iter()
    .enumerate()
    {
        list.push(DrawCommand::Text {
            rect: Rect {
                x: 20,
                y: 72 + index as u32 * 38,
                width: 170,
                height: 24,
            },
            content: family.to_owned(),
            role: "control".to_owned(),
            color: if index == 0 { text } else { soft },
        });
    }
    let content_x = sidebar_width + 28;
    let card_width = (viewport.width - content_x - 28 - 16) / 2;
    for (index, kind) in ComponentKind::ALL.into_iter().enumerate() {
        let column = index % 2;
        let row = index / 2;
        let x = content_x + column as u32 * (card_width + 16);
        let y = 76 + row as u32 * 98;
        let rect = Rect {
            x,
            y,
            width: card_width,
            height: 84,
        };
        frame(&mut list, rect, 9, layer, line);
        list.push(DrawCommand::Text {
            rect: Rect {
                x: x + 12,
                y: y + 10,
                width: card_width.saturating_sub(24),
                height: 22,
            },
            content: format!("{kind:?}"),
            role: "control".to_owned(),
            color: text,
        });
        list.push(DrawCommand::Text {
            rect: Rect {
                x: x + 12,
                y: y + 38,
                width: card_width.saturating_sub(24),
                height: 18,
            },
            content: format!("{} · default · focus · disabled", kind.family()),
            role: "caption".to_owned(),
            color: soft,
        });
        list.push(DrawCommand::Line {
            from: (x + 12, y + 70),
            to: (x + card_width.saturating_sub(12), y + 70),
            color: if kind == ComponentKind::NativePortal {
                token_color(theme, "aos.color.warning", [242, 181, 89, 255])
            } else {
                accent
            },
            width: 2,
        });
    }
    // Keep the retained semantic layer last in painter order.  The canvas,
    // sidebar, and card fills must never overdraw it.
    semantic_display(viewport, theme, &ShellState::default(), root, &mut list);
    list
}

fn frame(list: &mut DisplayList, rect: Rect, radius: u16, fill: Color, line: Color) {
    list.push(DrawCommand::FillRoundRect {
        rect,
        radius,
        color: fill,
    });
    list.push(DrawCommand::StrokeRoundRect {
        rect,
        radius,
        color: line,
        width: 1,
    });
}

/// Build a fixture with default settings and return its snapshot.
pub fn render_fixture(kind: FixtureKind, config: ThemeConfig) -> Result<Snapshot, FixtureError> {
    Fixture::new(kind, config).map(|fixture| fixture.snapshot())
}

/// Verify that building the same fixture twice yields the same semantic and display digests.
pub fn replay_digest(kind: FixtureKind, config: ThemeConfig) -> Result<bool, FixtureError> {
    let left = Fixture::new(kind, config)?;
    let right = Fixture::new(kind, config)?;
    Ok(left.snapshot() == right.snapshot())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ThemeConfig;

    fn reviewed_patch(
        fixture: &Fixture,
        id: &str,
        operations: Vec<crate::activity::PatchOp>,
    ) -> Patch {
        Patch {
            schema: "aos.patch@1".to_owned(),
            owner_ref: fixture.activity.owner_ref.clone(),
            acting_principal: crate::activity::OpaquePrincipalRef::Agent("agent".to_owned()),
            proposal_id: "proposal".to_owned(),
            review: crate::activity::ReviewAcceptance {
                reviewer: crate::activity::OpaquePrincipalRef::User("reviewer".to_owned()),
                receipt: "review".to_owned(),
            },
            patch_id: id.to_owned(),
            recipe_id: fixture.recipe.recipe_id.clone(),
            base_revision: fixture.recipe.revision,
            base_digest: fixture.recipe.digest.clone(),
            summary: "reviewed semantic change".to_owned(),
            operations,
        }
    }

    #[test]
    fn snapshots_are_replayable() {
        assert!(replay_digest(FixtureKind::Desktop, ThemeConfig::default()).expect("fixture"));
        assert!(replay_digest(FixtureKind::Phone, ThemeConfig::default()).expect("fixture"));
        assert!(replay_digest(FixtureKind::ThemeLab, ThemeConfig::default()).expect("fixture"));
    }

    #[test]
    fn phone_has_one_visible_surface_and_theme_lab_covers_catalog() {
        let phone = Fixture::new(FixtureKind::Phone, ThemeConfig::default()).expect("fixture");
        assert_eq!(phone.snapshot().visible_surface_count, 1);
        let lab = Fixture::new(FixtureKind::ThemeLab, ThemeConfig::default()).expect("fixture");
        assert_eq!(lab.surface.root.children[0].children.len(), 62);
    }

    #[test]
    fn accepted_patch_rematerializes_surface_and_advances_incarnation() {
        let mut fixture =
            Fixture::new(FixtureKind::Desktop, ThemeConfig::default()).expect("fixture");
        let root_id = fixture.recipe.root.id;
        let patch = Patch {
            schema: "aos.patch@1".to_owned(),
            owner_ref: fixture.activity.owner_ref.clone(),
            acting_principal: crate::activity::OpaquePrincipalRef::Agent(
                "fixture-agent".to_owned(),
            ),
            proposal_id: "fixture-proposal".to_owned(),
            review: crate::activity::ReviewAcceptance {
                reviewer: crate::activity::OpaquePrincipalRef::User("fixture-reviewer".to_owned()),
                receipt: "fixture-review".to_owned(),
            },
            patch_id: "fixture-patch".to_owned(),
            recipe_id: fixture.recipe.recipe_id.clone(),
            base_revision: fixture.recipe.revision,
            base_digest: fixture.recipe.digest.clone(),
            summary: "focus the activity".to_owned(),
            operations: vec![crate::activity::PatchOp::SetState {
                node_id: root_id,
                state: StateSet::empty().with(StateSet::FOCUS, true),
            }],
        };
        let outcome = fixture.apply_patch(&patch).expect("accepted");
        assert!(matches!(
            outcome,
            PatchOutcome::Applied {
                visual_changed: true,
                ..
            }
        ));
        assert_eq!(fixture.recipe.revision, 2);
        assert_eq!(fixture.surface.incarnation, 2);
        assert!(fixture.activity.current_surface.is_some());
    }

    #[test]
    fn visual_patch_changes_display_and_nonvisual_patch_does_not() {
        let mut fixture = Fixture::new(FixtureKind::Desktop, ThemeConfig::default()).unwrap();
        let before = fixture.snapshot().display.digest;
        let visual = reviewed_patch(
            &fixture,
            "visual",
            vec![crate::activity::PatchOp::SetState {
                node_id: fixture.recipe.root.id,
                state: StateSet::empty().with(StateSet::FOCUS, true),
            }],
        );
        assert!(matches!(
            fixture.apply_patch(&visual).unwrap(),
            PatchOutcome::Applied {
                visual_changed: true,
                ..
            }
        ));
        assert_ne!(before, fixture.snapshot().display.digest);
        assert_eq!(fixture.surface.incarnation, 2);

        let mut fixture = Fixture::new(FixtureKind::Desktop, ThemeConfig::default()).unwrap();
        let before = fixture.snapshot().display.digest;
        let nonvisual = reviewed_patch(
            &fixture,
            "nonvisual",
            vec![crate::activity::PatchOp::SetRecipeMetadata {
                key: "restore_hint".to_owned(),
                value: PropValue::Text("anchor".to_owned()),
            }],
        );
        assert!(matches!(
            fixture.apply_patch(&nonvisual).unwrap(),
            PatchOutcome::Applied {
                visual_changed: false,
                ..
            }
        ));
        assert_eq!(before, fixture.snapshot().display.digest);
        assert_eq!(fixture.recipe.revision, 2);
        assert_eq!(fixture.surface.incarnation, 1);
    }

    #[test]
    fn opaque_fill_cannot_cover_semantic_sample_pixel() {
        for kind in [FixtureKind::Desktop, FixtureKind::ThemeLab] {
            let fixture = Fixture::new(kind, ThemeConfig::default()).expect("fixture");
            let root_name = fixture.surface.root.accessibility.name.as_str();
            let list = fixture.display_list();
            let (semantic_index, semantic_rect) = list
                .commands
                .iter()
                .enumerate()
                .find_map(|(index, command)| match command {
                    DrawCommand::Text {
                        rect,
                        content,
                        role,
                        ..
                    } if role == "control" && content == root_name => Some((index, *rect)),
                    _ => None,
                })
                .expect("root semantic text command");
            let sample = (
                semantic_rect.x + semantic_rect.width / 2,
                semantic_rect.y + semantic_rect.height / 2,
            );
            let later_opaque_fill = list.commands.iter().enumerate().any(|(index, command)| {
                if index <= semantic_index {
                    return false;
                }
                match command {
                    DrawCommand::FillRoundRect { rect, color, .. } => {
                        color[3] == u8::MAX
                            && sample.0 >= rect.x
                            && sample.1 >= rect.y
                            && sample.0 < rect.x.saturating_add(rect.width)
                            && sample.1 < rect.y.saturating_add(rect.height)
                    }
                    _ => false,
                }
            });
            assert!(
                !later_opaque_fill,
                "{kind} has an opaque fill after the semantic root sample pixel"
            );
        }
    }
}
