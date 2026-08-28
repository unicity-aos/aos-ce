//! Declared-loss A2UI input mapping evidence owned by the catalog.

use crate::catalog::Primitive;
use std::fmt;

/// One declared-loss source-to-catalog mapping.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeclaredLossMapping {
    /// A2UI source identity.
    pub source: &'static str,
    /// Permitted catalog targets, in preference order.
    pub targets: &'static [Primitive],
}

/// A2UI mapping failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum A2uiImportError {
    /// Source had no declared-loss catalog mapping.
    UnmappedSource,
}

impl fmt::Display for A2uiImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("A2UI component has no declared-loss catalog mapping")
    }
}

impl std::error::Error for A2uiImportError {}

/// Normative mapping consumed by the #91 adapter.
pub const A2UI_DECLARED_LOSS: &[DeclaredLossMapping] = &[
    DeclaredLossMapping {
        source: "Row",
        targets: &[Primitive::Stack],
    },
    DeclaredLossMapping {
        source: "Column",
        targets: &[Primitive::Stack],
    },
    DeclaredLossMapping {
        source: "List",
        targets: &[Primitive::Repeater, Primitive::Stack],
    },
    DeclaredLossMapping {
        source: "ChoicePicker",
        targets: &[Primitive::Select, Primitive::MultiSelect],
    },
    DeclaredLossMapping {
        source: "Modal",
        targets: &[Primitive::Dialog],
    },
    DeclaredLossMapping {
        source: "Image",
        targets: &[Primitive::ImageView],
    },
    DeclaredLossMapping {
        source: "TextHeadingHint",
        targets: &[Primitive::Heading, Primitive::Text],
    },
    DeclaredLossMapping {
        source: "TextBodyHint",
        targets: &[Primitive::Text, Primitive::Heading],
    },
];

/// Return all declared catalog targets, preserving preference order.
pub fn map_a2ui_component(source: &str) -> Result<&'static [Primitive], A2uiImportError> {
    A2UI_DECLARED_LOSS
        .iter()
        .find(|mapping| mapping.source == source)
        .map(|mapping| mapping.targets)
        .ok_or(A2uiImportError::UnmappedSource)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_every_required_a2ui_boundary() {
        assert_eq!(map_a2ui_component("Row").unwrap(), &[Primitive::Stack][..]);
        assert_eq!(
            map_a2ui_component("Column").unwrap(),
            &[Primitive::Stack][..]
        );
        assert_eq!(
            map_a2ui_component("List").unwrap(),
            &[Primitive::Repeater, Primitive::Stack][..]
        );
        assert_eq!(
            map_a2ui_component("ChoicePicker").unwrap(),
            &[Primitive::Select, Primitive::MultiSelect][..]
        );
        assert_eq!(
            map_a2ui_component("Modal").unwrap(),
            &[Primitive::Dialog][..]
        );
        assert_eq!(
            map_a2ui_component("Image").unwrap(),
            &[Primitive::ImageView][..]
        );
        assert_eq!(
            map_a2ui_component("TextHeadingHint").unwrap(),
            &[Primitive::Heading, Primitive::Text][..]
        );
        assert_eq!(
            map_a2ui_component("NotInA2ui"),
            Err(A2uiImportError::UnmappedSource)
        );
    }
}
