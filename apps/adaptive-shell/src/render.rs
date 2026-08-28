//! Backend-neutral display-list and renderer boundary.

use crate::layout::Rect;
use serde::{Deserialize, Serialize};
use std::fmt;

/// A backend-neutral color.
pub type Color = [u8; 4];

/// Primitive display commands consumed by a native renderer adapter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DrawCommand {
    /// Fill a rounded rectangle.
    FillRoundRect {
        /// Target rectangle.
        rect: Rect,
        /// Corner radius in logical pixels.
        radius: u16,
        /// Fill color.
        color: Color,
    },
    /// Stroke a rounded rectangle.
    StrokeRoundRect {
        /// Target rectangle.
        rect: Rect,
        /// Corner radius in logical pixels.
        radius: u16,
        /// Stroke color.
        color: Color,
        /// Stroke width.
        width: u16,
    },
    /// Draw a horizontal or vertical line.
    Line {
        /// Start point.
        from: (u32, u32),
        /// End point.
        to: (u32, u32),
        /// Line color.
        color: Color,
        /// Width.
        width: u16,
    },
    /// Draw semantic text.  A text backend can choose shaping and fallback.
    Text {
        /// Text bounds.
        rect: Rect,
        /// UTF-8 content.
        content: String,
        /// Semantic typography role.
        role: String,
        /// Text color.
        color: Color,
    },
    /// Draw an icon by semantic name.
    Icon {
        /// Icon bounds.
        rect: Rect,
        /// Registered icon name.
        name: String,
        /// Icon color.
        color: Color,
    },
    /// A bounded media or portal placeholder.
    Placeholder {
        /// Placeholder bounds.
        rect: Rect,
        /// Human-readable state.
        label: String,
        /// Accent color.
        color: Color,
    },
}

/// Ordered display commands for one deterministic frame.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DisplayList {
    /// Logical frame size.
    pub viewport: (u32, u32),
    /// Commands in painter order.
    pub commands: Vec<DrawCommand>,
}

impl DisplayList {
    /// Construct an empty list.
    pub const fn new(viewport: (u32, u32)) -> Self {
        Self {
            viewport,
            commands: Vec::new(),
        }
    }

    /// Append a command.
    pub fn push(&mut self, command: DrawCommand) {
        self.commands.push(command);
    }

    /// Stable digest suitable for snapshot assertions.
    pub fn digest(&self) -> String {
        let bytes = serde_json::to_vec(self).expect("display list is serializable");
        blake3::hash(&bytes).to_hex().to_string()
    }

    /// Return a compact summary for a human smoke run.
    pub fn summary(&self) -> DisplaySummary {
        let mut fills = 0;
        let mut strokes = 0;
        let mut text = 0;
        let mut icons = 0;
        let mut placeholders = 0;
        for command in &self.commands {
            match command {
                DrawCommand::FillRoundRect { .. } => fills += 1,
                DrawCommand::StrokeRoundRect { .. } | DrawCommand::Line { .. } => strokes += 1,
                DrawCommand::Text { .. } => text += 1,
                DrawCommand::Icon { .. } => icons += 1,
                DrawCommand::Placeholder { .. } => placeholders += 1,
            }
        }
        DisplaySummary {
            viewport: self.viewport,
            commands: self.commands.len(),
            fills,
            strokes,
            text,
            icons,
            placeholders,
            digest: self.digest(),
        }
    }
}

/// Compact command counts and digest.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DisplaySummary {
    /// Logical frame size.
    pub viewport: (u32, u32),
    /// Total command count.
    pub commands: usize,
    /// Rounded fills.
    pub fills: usize,
    /// Strokes and lines.
    pub strokes: usize,
    /// Text commands.
    pub text: usize,
    /// Icon commands.
    pub icons: usize,
    /// Placeholder commands.
    pub placeholders: usize,
    /// Stable frame digest.
    pub digest: String,
}

/// Render result from a backend adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderReceipt {
    /// Digest of the accepted display list.
    pub digest: String,
    /// Number of accepted commands.
    pub command_count: usize,
}

/// Renderer backend boundary.  Concrete GPU/window handles stay below this trait.
pub trait Renderer {
    /// Consume a display list.
    fn render(&mut self, display_list: &DisplayList) -> Result<RenderReceipt, RenderError>;
}

/// Deterministic headless renderer used by tests and the CLI smoke path.
#[derive(Clone, Debug, Default)]
pub struct HeadlessRenderer {
    last: Option<DisplayList>,
}

impl HeadlessRenderer {
    /// Construct an empty headless renderer.
    pub const fn new() -> Self {
        Self { last: None }
    }

    /// Return the last accepted list.
    pub fn last(&self) -> Option<&DisplayList> {
        self.last.as_ref()
    }
}

impl Renderer for HeadlessRenderer {
    fn render(&mut self, display_list: &DisplayList) -> Result<RenderReceipt, RenderError> {
        if display_list.viewport.0 == 0 || display_list.viewport.1 == 0 {
            return Err(RenderError::InvalidViewport);
        }
        let receipt = RenderReceipt {
            digest: display_list.digest(),
            command_count: display_list.commands.len(),
        };
        self.last = Some(display_list.clone());
        Ok(receipt)
    }
}

/// Renderer boundary failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenderError {
    /// A zero-sized frame cannot be rendered.
    InvalidViewport,
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidViewport => f.write_str("renderer viewport cannot be zero-sized"),
        }
    }
}

impl std::error::Error for RenderError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headless_render_is_deterministic() {
        let mut list = DisplayList::new((320, 240));
        list.push(DrawCommand::Text {
            rect: Rect {
                x: 1,
                y: 1,
                width: 100,
                height: 20,
            },
            content: "hello".to_owned(),
            role: "body".to_owned(),
            color: [255, 255, 255, 255],
        });
        let digest = list.digest();
        let mut renderer = HeadlessRenderer::new();
        let receipt = renderer.render(&list).expect("valid viewport");
        assert_eq!(receipt.digest, digest);
        assert_eq!(renderer.last(), Some(&list));
    }
}
