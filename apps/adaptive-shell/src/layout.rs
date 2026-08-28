//! Responsive desktop and phone layout policy.

use serde::{Deserialize, Serialize};

/// Width at which the desktop shell becomes a phone shell.
pub const PHONE_BREAKPOINT: u32 = 760;
/// Desktop top-bar height.
pub const DESKTOP_TOP_BAR: u32 = 44;
/// Phone activity strip height.
pub const PHONE_ACTIVITY_STRIP: u32 = 66;
/// Minimum and maximum master-column percentages.
pub const MASTER_MIN_PERCENT: u8 = 38;
/// Maximum master-column percentage.
pub const MASTER_MAX_PERCENT: u8 = 76;
/// Default master-column percentage.
pub const MASTER_DEFAULT_PERCENT: u8 = 62;

/// Workspace layout modes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LayoutMode {
    /// One master plus up to two stacked secondary surfaces.
    #[default]
    Master,
    /// Two equal surfaces.
    Grid,
    /// One surface fills the workspace.
    Single,
    /// One selected surface fills the workspace temporarily.
    Focus,
}

/// Logical viewport dimensions.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Viewport {
    /// Width in logical pixels.
    pub width: u32,
    /// Height in logical pixels.
    pub height: u32,
}

impl Viewport {
    /// Construct a viewport.
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// Whether phone mode applies.
    pub const fn is_phone(self) -> bool {
        self.width < PHONE_BREAKPOINT
    }
}

/// Integer rectangle used by the backend-neutral display list.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    /// Left coordinate.
    pub x: u32,
    /// Top coordinate.
    pub y: u32,
    /// Width.
    pub width: u32,
    /// Height.
    pub height: u32,
}

/// Semantic role of a laid-out slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SlotKind {
    /// Primary activity surface.
    Master,
    /// Secondary activity surface.
    Secondary,
    /// The sole phone surface.
    Phone,
}

/// One visible surface slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SurfaceSlot {
    /// Surface index in activity order.
    pub surface_index: usize,
    /// Slot role.
    pub kind: SlotKind,
    /// Pixel rectangle.
    pub rect: Rect,
}

/// Result of applying responsive layout policy.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LayoutPlan {
    /// Input viewport.
    pub viewport: Viewport,
    /// Resolved mode (phone always reports single visible surface).
    pub mode: LayoutMode,
    /// Whether phone-specific rules apply.
    pub phone: bool,
    /// Clamped master-column percentage.
    pub master_percent: u8,
    /// Number of secondary surfaces visible in the stack.
    pub stack_count: usize,
    /// Bottom strip height in this plan.
    pub activity_strip_height: u32,
    /// Visible surface slots in reading order.
    pub slots: Vec<SurfaceSlot>,
}

/// Stateless responsive layout resolver.
#[derive(Clone, Copy, Debug, Default)]
pub struct LayoutPolicy;

impl LayoutPolicy {
    /// Resolve a layout with at most two secondary surfaces visible.
    pub fn resolve(
        viewport: Viewport,
        mode: LayoutMode,
        requested_master_percent: u8,
        surface_count: usize,
    ) -> LayoutPlan {
        let phone = viewport.is_phone();
        if phone {
            let inset = 8;
            let top = 48;
            let bottom = PHONE_ACTIVITY_STRIP;
            let width = viewport.width.saturating_sub(inset * 2);
            let height = viewport.height.saturating_sub(top + bottom + inset);
            let slots = if surface_count == 0 {
                Vec::new()
            } else {
                vec![SurfaceSlot {
                    surface_index: 0,
                    kind: SlotKind::Phone,
                    rect: Rect {
                        x: inset,
                        y: top,
                        width,
                        height,
                    },
                }]
            };
            return LayoutPlan {
                viewport,
                mode: LayoutMode::Single,
                phone: true,
                master_percent: requested_master_percent
                    .clamp(MASTER_MIN_PERCENT, MASTER_MAX_PERCENT),
                stack_count: 0,
                activity_strip_height: PHONE_ACTIVITY_STRIP,
                slots,
            };
        }

        let master_percent = requested_master_percent.clamp(MASTER_MIN_PERCENT, MASTER_MAX_PERCENT);
        let inset = 10;
        let gap = 10;
        let top = DESKTOP_TOP_BAR + inset;
        let width = viewport.width.saturating_sub(inset * 2);
        let height = viewport.height.saturating_sub(top + inset);
        let visible = surface_count.min(3);
        let mut slots = Vec::new();
        let resolved_mode = if surface_count == 0 {
            LayoutMode::Single
        } else {
            mode
        };
        match resolved_mode {
            LayoutMode::Master => {
                if visible > 0 {
                    let master_width = width
                        .saturating_sub(gap)
                        .saturating_mul(u32::from(master_percent))
                        / 100;
                    slots.push(SurfaceSlot {
                        surface_index: 0,
                        kind: SlotKind::Master,
                        rect: Rect {
                            x: inset,
                            y: top,
                            width: master_width,
                            height,
                        },
                    });
                    let stack_x = inset + master_width + gap;
                    let stack_width = width.saturating_sub(master_width + gap);
                    let stack_count = visible.saturating_sub(1).min(2);
                    let stack_gap = if stack_count > 1 { gap } else { 0 };
                    let each_height = if stack_count == 0 {
                        0
                    } else {
                        height.saturating_sub(stack_gap) / stack_count as u32
                    };
                    for index in 0..stack_count {
                        slots.push(SurfaceSlot {
                            surface_index: index + 1,
                            kind: SlotKind::Secondary,
                            rect: Rect {
                                x: stack_x,
                                y: top + index as u32 * (each_height + stack_gap),
                                width: stack_width,
                                height: each_height,
                            },
                        });
                    }
                }
            }
            LayoutMode::Grid => {
                let count = visible.min(2);
                let each_width = if count == 0 {
                    0
                } else {
                    width.saturating_sub(gap) / count as u32
                };
                for index in 0..count {
                    slots.push(SurfaceSlot {
                        surface_index: index,
                        kind: if index == 0 {
                            SlotKind::Master
                        } else {
                            SlotKind::Secondary
                        },
                        rect: Rect {
                            x: inset + index as u32 * (each_width + gap),
                            y: top,
                            width: each_width,
                            height,
                        },
                    });
                }
            }
            LayoutMode::Single | LayoutMode::Focus => {
                if visible > 0 {
                    slots.push(SurfaceSlot {
                        surface_index: 0,
                        kind: SlotKind::Master,
                        rect: Rect {
                            x: inset,
                            y: top,
                            width,
                            height,
                        },
                    });
                }
            }
        }
        let stack_count = slots
            .iter()
            .filter(|slot| slot.kind == SlotKind::Secondary)
            .count();
        LayoutPlan {
            viewport,
            mode: resolved_mode,
            phone: false,
            master_percent,
            stack_count,
            activity_strip_height: 0,
            slots,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_master_clamps_and_caps_stack() {
        let plan = LayoutPolicy::resolve(Viewport::new(1440, 1000), LayoutMode::Master, 100, 5);
        assert!(!plan.phone);
        assert_eq!(plan.master_percent, MASTER_MAX_PERCENT);
        assert_eq!(plan.stack_count, 2);
        assert_eq!(plan.slots.len(), 3);
    }

    #[test]
    fn phone_is_one_surface_plus_strip() {
        let plan = LayoutPolicy::resolve(Viewport::new(390, 844), LayoutMode::Grid, 62, 4);
        assert!(plan.phone);
        assert_eq!(plan.mode, LayoutMode::Single);
        assert_eq!(plan.slots.len(), 1);
        assert_eq!(plan.activity_strip_height, PHONE_ACTIVITY_STRIP);
    }
}
