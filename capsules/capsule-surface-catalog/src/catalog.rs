//! The finite `aos.catalog/1` primitive inventory and record validator.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;

/// Stable catalog contract identity.
pub const CATALOG_SCHEMA: &str = "aos.catalog/1";

const MAX_TEXT_BYTES: usize = 320;
const MAX_SLOTS: usize = 8;

/// Every v1 primitive in the exact stable order owned by this capsule.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum Primitive {
    /// A named semantic region.
    Region,
    /// An ordered vertical or horizontal stack.
    Stack,
    /// A grid of semantic children.
    Grid,
    /// A resizable split.
    Split,
    /// Navigation sidebar.
    Sidebar,
    /// A row of semantic actions.
    ActionBar,
    /// Framed content card.
    Card,
    /// Preference or content group.
    Group,
    /// Collapsible content.
    Collapse,
    /// Repeated bounded children.
    Repeater,
    /// Scrollable semantic region.
    ScrollRegion,
    /// A semantic divider.
    Divider,
    /// A heading.
    Heading,
    /// Body or inline text.
    Text,
    /// Inline rich content.
    InlineContent,
    /// Source or code text.
    CodeBlock,
    /// Compact label.
    Badge,
    /// Prominent numeric fact.
    KeyFigure,
    /// Empty-result explanation.
    EmptyState,
    /// A named icon.
    Icon,
    /// An action control; icon-only controls use the icon slot.
    Button,
    /// Single-line text entry.
    TextField,
    /// Multi-line text entry.
    TextArea,
    /// Numeric entry.
    NumberField,
    /// Single selection.
    Select,
    /// Multiple selection.
    MultiSelect,
    /// Boolean switch.
    Switch,
    /// Boolean checkbox.
    Checkbox,
    /// Bounded range input.
    Slider,
    /// Date or time entry.
    DateTimeField,
    /// Tabular data.
    Table,
    /// Concise record summary.
    RecordSummary,
    /// Data chart with a textual equivalent.
    DatasetChart,
    /// Ordered events.
    Timeline,
    /// Before-and-after difference.
    Difference,
    /// Progress indicator.
    Progress,
    /// Tab navigation.
    Tabs,
    /// Hierarchical navigation.
    Breadcrumb,
    /// Context menu.
    Menu,
    /// Page navigation.
    Pager,
    /// A semantic link.
    Link,
    /// Important non-blocking alert.
    Alert,
    /// Temporary notification.
    Toast,
    /// Inline feedback.
    InlineMessage,
    /// Loading placeholder.
    Skeleton,
    /// Busy indicator.
    Spinner,
    /// Small status indicator.
    StatusDot,
    /// Modal dialog.
    Dialog,
    /// Capability description, never a grant.
    CapabilityCard,
    /// Consent request, never consent itself.
    ConsentForm,
    /// Deliberate secure-input prompt.
    SecurePrompt,
    /// Governed file-selection request.
    FilePicker,
    /// Image media.
    ImageView,
    /// Audio media.
    AudioPlayer,
    /// Video media.
    VideoPlayer,
    /// File metadata.
    FileDetails,
    /// Typed principal-scoped media reference.
    MediaEmbed,
    /// Free-form drawing stage.
    CanvasStage,
    /// Diagram surface.
    DiagramView,
    /// Annotation layer.
    AnnotationLayer,
    /// Presentation of a bound terminal session.
    TerminalView,
    /// Presentation of external portal state.
    NativePortal,
}

impl Primitive {
    /// Every primitive in stable contract order.
    pub const ALL: [Self; 62] = [
        Self::Region,
        Self::Stack,
        Self::Grid,
        Self::Split,
        Self::Sidebar,
        Self::ActionBar,
        Self::Card,
        Self::Group,
        Self::Collapse,
        Self::Repeater,
        Self::ScrollRegion,
        Self::Divider,
        Self::Heading,
        Self::Text,
        Self::InlineContent,
        Self::CodeBlock,
        Self::Badge,
        Self::KeyFigure,
        Self::EmptyState,
        Self::Icon,
        Self::Button,
        Self::TextField,
        Self::TextArea,
        Self::NumberField,
        Self::Select,
        Self::MultiSelect,
        Self::Switch,
        Self::Checkbox,
        Self::Slider,
        Self::DateTimeField,
        Self::Table,
        Self::RecordSummary,
        Self::DatasetChart,
        Self::Timeline,
        Self::Difference,
        Self::Progress,
        Self::Tabs,
        Self::Breadcrumb,
        Self::Menu,
        Self::Pager,
        Self::Link,
        Self::Alert,
        Self::Toast,
        Self::InlineMessage,
        Self::Skeleton,
        Self::Spinner,
        Self::StatusDot,
        Self::Dialog,
        Self::CapabilityCard,
        Self::ConsentForm,
        Self::SecurePrompt,
        Self::FilePicker,
        Self::ImageView,
        Self::AudioPlayer,
        Self::VideoPlayer,
        Self::FileDetails,
        Self::MediaEmbed,
        Self::CanvasStage,
        Self::DiagramView,
        Self::AnnotationLayer,
        Self::TerminalView,
        Self::NativePortal,
    ];

    /// Stable string identity.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Region => "Region",
            Self::Stack => "Stack",
            Self::Grid => "Grid",
            Self::Split => "Split",
            Self::Sidebar => "Sidebar",
            Self::ActionBar => "ActionBar",
            Self::Card => "Card",
            Self::Group => "Group",
            Self::Collapse => "Collapse",
            Self::Repeater => "Repeater",
            Self::ScrollRegion => "ScrollRegion",
            Self::Divider => "Divider",
            Self::Heading => "Heading",
            Self::Text => "Text",
            Self::InlineContent => "InlineContent",
            Self::CodeBlock => "CodeBlock",
            Self::Badge => "Badge",
            Self::KeyFigure => "KeyFigure",
            Self::EmptyState => "EmptyState",
            Self::Icon => "Icon",
            Self::Button => "Button",
            Self::TextField => "TextField",
            Self::TextArea => "TextArea",
            Self::NumberField => "NumberField",
            Self::Select => "Select",
            Self::MultiSelect => "MultiSelect",
            Self::Switch => "Switch",
            Self::Checkbox => "Checkbox",
            Self::Slider => "Slider",
            Self::DateTimeField => "DateTimeField",
            Self::Table => "Table",
            Self::RecordSummary => "RecordSummary",
            Self::DatasetChart => "DatasetChart",
            Self::Timeline => "Timeline",
            Self::Difference => "Difference",
            Self::Progress => "Progress",
            Self::Tabs => "Tabs",
            Self::Breadcrumb => "Breadcrumb",
            Self::Menu => "Menu",
            Self::Pager => "Pager",
            Self::Link => "Link",
            Self::Alert => "Alert",
            Self::Toast => "Toast",
            Self::InlineMessage => "InlineMessage",
            Self::Skeleton => "Skeleton",
            Self::Spinner => "Spinner",
            Self::StatusDot => "StatusDot",
            Self::Dialog => "Dialog",
            Self::CapabilityCard => "CapabilityCard",
            Self::ConsentForm => "ConsentForm",
            Self::SecurePrompt => "SecurePrompt",
            Self::FilePicker => "FilePicker",
            Self::ImageView => "ImageView",
            Self::AudioPlayer => "AudioPlayer",
            Self::VideoPlayer => "VideoPlayer",
            Self::FileDetails => "FileDetails",
            Self::MediaEmbed => "MediaEmbed",
            Self::CanvasStage => "CanvasStage",
            Self::DiagramView => "DiagramView",
            Self::AnnotationLayer => "AnnotationLayer",
            Self::TerminalView => "TerminalView",
            Self::NativePortal => "NativePortal",
        }
    }

    /// Parse the stable string identity without accepting extensions.
    pub fn from_id(value: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|kind| kind.as_str() == value)
    }

    /// Coarse semantic family.
    pub const fn family(self) -> PrimitiveFamily {
        match self {
            Self::Region
            | Self::Stack
            | Self::Grid
            | Self::Split
            | Self::Sidebar
            | Self::ActionBar
            | Self::Card
            | Self::Group
            | Self::Collapse
            | Self::Repeater
            | Self::ScrollRegion
            | Self::Divider => PrimitiveFamily::Layout,
            Self::Heading
            | Self::Text
            | Self::InlineContent
            | Self::CodeBlock
            | Self::Badge
            | Self::KeyFigure
            | Self::EmptyState
            | Self::Icon => PrimitiveFamily::Content,
            Self::Button
            | Self::TextField
            | Self::TextArea
            | Self::NumberField
            | Self::Select
            | Self::MultiSelect
            | Self::Switch
            | Self::Checkbox
            | Self::Slider
            | Self::DateTimeField => PrimitiveFamily::Input,
            Self::Table
            | Self::RecordSummary
            | Self::DatasetChart
            | Self::Timeline
            | Self::Difference
            | Self::Progress => PrimitiveFamily::Data,
            Self::Tabs | Self::Breadcrumb | Self::Menu | Self::Pager | Self::Link => {
                PrimitiveFamily::Navigation
            }
            Self::Alert
            | Self::Toast
            | Self::InlineMessage
            | Self::Skeleton
            | Self::Spinner
            | Self::StatusDot
            | Self::Dialog => PrimitiveFamily::Feedback,
            Self::CapabilityCard | Self::ConsentForm | Self::SecurePrompt | Self::FilePicker => {
                PrimitiveFamily::Permission
            }
            Self::ImageView
            | Self::AudioPlayer
            | Self::VideoPlayer
            | Self::FileDetails
            | Self::MediaEmbed => PrimitiveFamily::Media,
            Self::CanvasStage | Self::DiagramView | Self::AnnotationLayer => {
                PrimitiveFamily::Canvas
            }
            Self::TerminalView => PrimitiveFamily::Terminal,
            Self::NativePortal => PrimitiveFamily::NativePortal,
        }
    }

    /// Whether presentation of this primitive can create authority.
    pub const fn mints_capability(self) -> bool {
        false
    }

    /// States that a complete record and Lab corpus must document.
    pub fn required_states(self) -> Vec<State> {
        let interaction = [
            State::Default,
            State::Hover,
            State::Pressed,
            State::FocusVisible,
            State::Disabled,
        ];
        let status = [State::Default, State::Loading, State::Empty, State::Error];
        match self {
            Self::Region
            | Self::Stack
            | Self::Grid
            | Self::Split
            | Self::Sidebar
            | Self::Card
            | Self::Group
            | Self::ScrollRegion
            | Self::Heading
            | Self::Text
            | Self::InlineContent
            | Self::CodeBlock
            | Self::Icon => vec![State::Default],
            Self::ActionBar
            | Self::Collapse
            | Self::Repeater
            | Self::Divider
            | Self::Badge
            | Self::KeyFigure
            | Self::EmptyState
            | Self::Table
            | Self::RecordSummary
            | Self::DatasetChart
            | Self::Timeline
            | Self::Difference
            | Self::Progress
            | Self::Tabs
            | Self::Breadcrumb
            | Self::Menu
            | Self::Pager
            | Self::Link => {
                let mut states = vec![State::Default, State::FocusVisible, State::Disabled];
                if matches!(
                    self,
                    Self::Repeater
                        | Self::EmptyState
                        | Self::Table
                        | Self::DatasetChart
                        | Self::Progress
                ) {
                    states.extend([State::Loading, State::Empty]);
                }
                states
            }
            Self::Button
            | Self::TextField
            | Self::TextArea
            | Self::NumberField
            | Self::Select
            | Self::MultiSelect
            | Self::Switch
            | Self::Checkbox
            | Self::Slider
            | Self::DateTimeField => interaction.to_vec(),
            Self::Alert | Self::Toast | Self::InlineMessage | Self::Dialog => vec![
                State::Default,
                State::FocusVisible,
                State::Disabled,
                State::Success,
                State::Warning,
                State::Danger,
            ],
            Self::Skeleton | Self::Spinner | Self::StatusDot => {
                vec![State::Default, State::Loading]
            }
            Self::CapabilityCard | Self::ConsentForm | Self::SecurePrompt | Self::FilePicker => {
                vec![
                    State::Default,
                    State::FocusVisible,
                    State::Disabled,
                    State::Loading,
                    State::Error,
                ]
            }
            Self::ImageView
            | Self::AudioPlayer
            | Self::VideoPlayer
            | Self::FileDetails
            | Self::MediaEmbed
            | Self::CanvasStage
            | Self::DiagramView
            | Self::AnnotationLayer => {
                let mut states = vec![State::Default, State::FocusVisible];
                states.extend(status[1..].iter().copied());
                states
            }
            Self::TerminalView => vec![
                State::Default,
                State::FocusVisible,
                State::Loading,
                State::Error,
            ],
            Self::NativePortal => vec![State::Default, State::Loading, State::Empty, State::Error],
        }
    }
}

impl fmt::Display for Primitive {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Coarse owned family.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PrimitiveFamily {
    /// Layout primitives.
    Layout,
    /// Content primitives.
    Content,
    /// Input controls.
    Input,
    /// Data presentation.
    Data,
    /// Navigation controls.
    Navigation,
    /// Feedback surfaces.
    Feedback,
    /// Authority-request visualizations.
    Permission,
    /// Media references.
    Media,
    /// Canvas surfaces.
    Canvas,
    /// Bound terminal presentation.
    Terminal,
    /// External portal presentation.
    NativePortal,
}

/// Declared child and slot policy.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ChildrenPolicy {
    /// The primitive owns no children or slots.
    None,
    /// A finite set of named slots.
    ClosedSlots {
        /// Allowed slot identifiers.
        slots: Vec<String>,
    },
    /// A bounded sequence of ordinary catalog children.
    BoundedChildren {
        /// Human-readable child role.
        child_role: String,
        /// Inclusive minimum.
        minimum: u8,
        /// Inclusive maximum.
        maximum: u8,
    },
}

/// Semantic interaction state.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum State {
    /// Resting presentation.
    Default,
    /// Pointer hover.
    Hover,
    /// Active press.
    Pressed,
    /// Keyboard-originated focus indicator.
    FocusVisible,
    /// Noninteractive or unavailable.
    Disabled,
    /// Work is pending.
    Loading,
    /// No content is available.
    Empty,
    /// A recoverable failure.
    Error,
    /// Positive completion.
    Success,
    /// Cautionary status.
    Warning,
    /// Severe status.
    Danger,
    /// Chosen item.
    Selected,
}

/// One complete v1 primitive record.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrimitiveRecord {
    /// Stable primitive identity.
    pub id: Primitive,
    /// Stable semantic role.
    pub semantic_role: String,
    /// Allowed children or slots.
    pub children: ChildrenPolicy,
    /// Required accessibility naming strategy.
    pub accessibility_label_strategy: String,
    /// Required state set.
    pub states: BTreeSet<State>,
    /// Density behavior expressed on the semantic scale.
    pub density_behavior: String,
    /// Compact phone adaptation.
    pub phone_adaptation: String,
    /// Desktop adaptation.
    pub desktop_adaptation: String,
    /// Native semantic mapping.
    pub native_mapping: String,
    /// Focus and keyboard contract.
    pub focus_keyboard_contract: String,
    /// Motion contract.
    pub motion_contract: String,
    /// Safe presentation fallback.
    pub fallback_rendering: String,
}

/// Catalog validation failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CatalogError {
    /// Contract identity was not exactly `aos.catalog/1`.
    Schema,
    /// The v1 inventory count was not exactly 62.
    PrimitiveCount,
    /// The records were absent, reordered, or duplicated.
    PrimitiveOrder,
    /// A required semantic field was empty or oversized.
    RecordField,
    /// A closed child contract had no slot or exceeded the slot bound.
    ChildrenPolicy,
}

impl fmt::Display for CatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Schema => "catalog schema must be aos.catalog/1",
            Self::PrimitiveCount => "catalog must own exactly 62 v1 primitives",
            Self::PrimitiveOrder => "catalog records must equal the stable v1 inventory",
            Self::RecordField => "catalog record has an invalid semantic field",
            Self::ChildrenPolicy => "catalog record has an invalid child policy",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for CatalogError {}

/// Canonical v1 catalog.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    /// Stable contract identity.
    pub schema: String,
    /// One record per v1 primitive.
    pub records: Vec<PrimitiveRecord>,
}

impl Catalog {
    /// Construct the canonical catalog.
    pub fn v1() -> Self {
        Self {
            schema: CATALOG_SCHEMA.to_owned(),
            records: Primitive::ALL
                .into_iter()
                .map(PrimitiveRecord::for_primitive)
                .collect(),
        }
    }

    /// Validate exact v1 identity, inventory, order, and record completeness.
    pub fn validate(&self) -> Result<(), CatalogError> {
        if self.schema != CATALOG_SCHEMA {
            return Err(CatalogError::Schema);
        }
        if self.records.len() != 62 {
            return Err(CatalogError::PrimitiveCount);
        }
        let expected: Vec<_> = Primitive::ALL.into_iter().collect();
        let actual: Vec<_> = self.records.iter().map(|record| record.id).collect();
        if actual != expected {
            return Err(CatalogError::PrimitiveOrder);
        }
        self.records
            .iter()
            .try_for_each(PrimitiveRecord::validate)?;
        Ok(())
    }

    /// Look up a record by primitive identity.
    pub fn record(&self, id: Primitive) -> Option<&PrimitiveRecord> {
        self.records.iter().find(|record| record.id == id)
    }
}

impl PrimitiveRecord {
    /// Build the normative record for a v1 primitive.
    pub fn for_primitive(id: Primitive) -> Self {
        let family = id.family();
        let (semantic_role, accessibility, focus, native, motion, fallback) = match family {
            PrimitiveFamily::Layout => (
                "layout container",
                "name a region that changes reading order; otherwise inherit the nearest named ancestor",
                "children manage focus; the container is not a tab stop unless explicitly scrollable",
                "map to the host composition container",
                "inherit reduced-motion policy; never animate layout autonomously",
                "render children in source order with default block layout",
            ),
            PrimitiveFamily::Content => (
                "content presentation",
                "use authored text as the accessible name; icons require an explicit text alternative",
                "selectable or link-like content may receive keyboard focus; ordinary text does not",
                "map to platform text or static-image semantics",
                "use opacity transitions only and suppress them when reduced motion is requested",
                "render plain text or descriptive metadata without replacing semantic content",
            ),
            PrimitiveFamily::Input => (
                "input control",
                "require an explicit programmatic label even when a visible label is absent",
                "make the control focusable and document Enter, Space, arrows, or Escape behavior",
                "map to the platform semantic control",
                "suppress value-change motion and use instant feedback when reduced motion is requested",
                "render the current value and label as read-only text with actions unavailable",
            ),
            PrimitiveFamily::Data => (
                "data presentation",
                "supply a concise programmatic name and text equivalent for the represented facts",
                "make row, tab, or summary interaction reachable in reading order",
                "map to platform list, table, image, or progress semantics with text alternatives",
                "respect reduced motion for transitions and provide a static final state",
                "render a bounded textual summary or empty-state explanation",
            ),
            PrimitiveFamily::Navigation => (
                "navigation control",
                "require a destination or section name distinct from surrounding text",
                "use roving or sequential keyboard traversal appropriate to the control and Enter activation",
                "map to platform navigation or link semantics",
                "use short transition only and set it to zero when reduced motion is requested",
                "render readable links or a flat list without performing navigation",
            ),
            PrimitiveFamily::Feedback => (
                "feedback presentation",
                "announce meaningful state with a programmatic name and concise live message",
                "make dismiss or acknowledgment controls reachable and Escape-close modal feedback",
                "map to platform alert, progress, dialog, or static-indicator semantics",
                "respect reduced motion; essential state changes remain visible without movement",
                "render static state text and keep dismissal unavailable rather than losing the message",
            ),
            PrimitiveFamily::Permission => (
                "authority request visualization",
                "name the requested scope and state that presentation is not approval",
                "make every affordance keyboard reachable; record explicit activation as a request",
                "map to static semantic content and route the request through policy, never presentation",
                "suppress attention motion under reduced motion and preserve the full request text",
                "render a read-only explanation and mark request actions unavailable",
            ),
            PrimitiveFamily::Media => (
                "media reference",
                "require alt text, transcript, title, or typed metadata before presentation",
                "make transport controls keyboard operable; metadata-only media need not be focusable",
                "map to platform media or static metadata semantics without arbitrary execution",
                "respect reduced motion and never autoplay moving or sounding media",
                "render typed metadata, alt text, or transcript without invoking the media reference",
            ),
            PrimitiveFamily::Canvas => (
                "authorable visual surface",
                "supply a programmatic name and nonvisual equivalent or annotation",
                "make tools and annotations keyboard reachable and trap pointer drawing only to the stage",
                "map to an opaque drawing or diagram view with an accessibility layer",
                "scale redraw motion to density and honor reduced motion with final-frame rendering",
                "render title, annotations, or textual equivalent without arbitrary stage execution",
            ),
            PrimitiveFamily::Terminal => (
                "bound terminal session presentation",
                "name the bound session and summarize its state; presentation is not a shell",
                "support focus, arrow scrolling, copy shortcuts, and no command entry from this view",
                "map to a read-only or bound-session native view supplied by policy",
                "avoid scroll animation and respect reduced motion",
                "render bounded session metadata and status without starting or sending a command",
            ),
            PrimitiveFamily::NativePortal => (
                "native portal presentation",
                "name the portal contract and display explicit unavailable or loaded state",
                "treat embedded controls as unavailable until a policy-approved native adapter owns them",
                "present portal state only; live native control remains outside this catalog",
                "show deterministic state changes without autonomous motion",
                "render a neutral unavailable state and preserve the declared portal identity",
            ),
        };
        let density = match family {
            PrimitiveFamily::Layout => {
                "compress spacing ratios on compact density and expand ratios on spacious density"
            }
            PrimitiveFamily::Content => {
                "scale typography by bounded ratios while preserving reading measure"
            }
            PrimitiveFamily::Input => {
                "preserve bounded minimum target height and adjust padding only"
            }
            PrimitiveFamily::Data => {
                "reduce row and column padding before truncating semantic text"
            }
            PrimitiveFamily::Navigation => {
                "keep reachable target height and collapse secondary labels on phone"
            }
            PrimitiveFamily::Feedback => "adjust padding and text scale without hiding the message",
            PrimitiveFamily::Permission => "preserve complete request text at every density",
            PrimitiveFamily::Media => "preserve aspect ratio and controls while reducing padding",
            PrimitiveFamily::Canvas => {
                "preserve semantic tool target size while scaling canvas bounds"
            }
            PrimitiveFamily::Terminal => "preserve legible monospace scale while reducing padding",
            PrimitiveFamily::NativePortal => {
                "preserve the declared portal bounds and padding ratios"
            }
        };
        let phone = match family {
            PrimitiveFamily::Layout => "stack children in one column with safe-area-aware spacing",
            PrimitiveFamily::Content => "wrap naturally and allow bounded vertical growth",
            PrimitiveFamily::Input => "use full-width controls with platform-native input surfaces",
            PrimitiveFamily::Data => {
                "summarize or progressively disclose columns while keeping text equivalents"
            }
            PrimitiveFamily::Navigation => "collapse to bottom, overflow, or compact controls",
            PrimitiveFamily::Feedback => {
                "present inline or full-width feedback with dismissal reachable"
            }
            PrimitiveFamily::Permission => "present one complete request in reading order",
            PrimitiveFamily::Media => "present controls below media and respect safe areas",
            PrimitiveFamily::Canvas => "use one full-width stage with reachable tools",
            PrimitiveFamily::Terminal => "use full width, bounded height, and scrollable output",
            PrimitiveFamily::NativePortal => "use a full-width unavailable or presentation frame",
        };
        let desktop = match family {
            PrimitiveFamily::Layout => "use ordered columns, grids, or splits at bounded ratios",
            PrimitiveFamily::Content => "use a bounded measure and desktop typography hierarchy",
            PrimitiveFamily::Input => {
                "use aligned controls and adjacent labels with pointer precision"
            }
            PrimitiveFamily::Data => {
                "use comparable columns or expanded visual form with text equivalents"
            }
            PrimitiveFamily::Navigation => {
                "use horizontal or persistent navigation with keyboard paths"
            }
            PrimitiveFamily::Feedback => {
                "use positioned, inline, or modal feedback with stable targets"
            }
            PrimitiveFamily::Permission => {
                "use a focused request layout while keeping policy text complete"
            }
            PrimitiveFamily::Media => {
                "use bounded media proportions with adjacent transport controls"
            }
            PrimitiveFamily::Canvas => "use a bounded stage with persistent tool palettes",
            PrimitiveFamily::Terminal => {
                "use a resizable bounded pane with persistent scrollbar semantics"
            }
            PrimitiveFamily::NativePortal => {
                "use declared desktop bounds and explicit state presentation"
            }
        };
        Self {
            id,
            semantic_role: semantic_role.to_owned(),
            children: Self::children(id),
            accessibility_label_strategy: accessibility.to_owned(),
            states: id.required_states().into_iter().collect(),
            density_behavior: density.to_owned(),
            phone_adaptation: phone.to_owned(),
            desktop_adaptation: desktop.to_owned(),
            native_mapping: native.to_owned(),
            focus_keyboard_contract: focus.to_owned(),
            motion_contract: motion.to_owned(),
            fallback_rendering: fallback.to_owned(),
        }
    }

    fn children(id: Primitive) -> ChildrenPolicy {
        let slots = |values: &[&str]| ChildrenPolicy::ClosedSlots {
            slots: values.iter().map(|value| (*value).to_owned()).collect(),
        };
        let children = |role: &str, minimum: u8, maximum: u8| ChildrenPolicy::BoundedChildren {
            child_role: role.to_owned(),
            minimum,
            maximum,
        };
        match id {
            Primitive::Split => slots(&["start", "end"]),
            Primitive::Card => slots(&["header", "content", "footer"]),
            Primitive::Collapse => slots(&["summary", "content"]),
            Primitive::Repeater => slots(&["template"]),
            Primitive::Button => slots(&["icon", "label"]),
            Primitive::Dialog => slots(&["header", "content", "footer"]),
            Primitive::Tabs => slots(&["tab", "panel"]),
            Primitive::Breadcrumb => children("crumb", 1, 16),
            Primitive::Menu => children("item", 0, 64),
            Primitive::Pager => slots(&["previous", "status", "next"]),
            Primitive::CapabilityCard => slots(&["scope-summary", "policy-links"]),
            Primitive::ConsentForm => slots(&["statement", "choices", "actions"]),
            Primitive::SecurePrompt => slots(&["prompt", "secure-input"]),
            Primitive::FilePicker => slots(&["filter-summary", "selection-preview"]),
            Primitive::DiagramView => slots(&["stage", "legend"]),
            Primitive::AnnotationLayer => children("annotation", 0, 64),
            id if Self::is_repeat_layout(id) => children("catalog primitive", 0, 64),
            _ => ChildrenPolicy::None,
        }
    }

    const fn is_repeat_layout(id: Primitive) -> bool {
        matches!(
            id,
            Primitive::Region
                | Primitive::Stack
                | Primitive::Grid
                | Primitive::Sidebar
                | Primitive::ActionBar
                | Primitive::Group
                | Primitive::ScrollRegion
        )
    }

    /// Validate completeness and bounded semantic text.
    pub fn validate(&self) -> Result<(), CatalogError> {
        let valid_children = match &self.children {
            ChildrenPolicy::None => true,
            ChildrenPolicy::ClosedSlots { slots } => {
                !slots.is_empty()
                    && slots.len() <= MAX_SLOTS
                    && slots.iter().all(|slot| bounded_text(slot))
            }
            ChildrenPolicy::BoundedChildren {
                child_role,
                minimum,
                maximum,
            } => bounded_text(child_role) && minimum <= maximum,
        };
        if !valid_children {
            return Err(CatalogError::ChildrenPolicy);
        }
        let fields = [
            &self.semantic_role,
            &self.accessibility_label_strategy,
            &self.density_behavior,
            &self.phone_adaptation,
            &self.desktop_adaptation,
            &self.native_mapping,
            &self.focus_keyboard_contract,
            &self.motion_contract,
            &self.fallback_rendering,
        ];
        if fields.iter().all(|field| bounded_text(field)) {
            Ok(())
        } else {
            Err(CatalogError::RecordField)
        }
    }
}

fn bounded_text(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= MAX_TEXT_BYTES
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXPECTED: [Primitive; 62] = Primitive::ALL;

    #[test]
    fn owns_the_exact_v1_inventory() {
        let expected_names = [
            "Region",
            "Stack",
            "Grid",
            "Split",
            "Sidebar",
            "ActionBar",
            "Card",
            "Group",
            "Collapse",
            "Repeater",
            "ScrollRegion",
            "Divider",
            "Heading",
            "Text",
            "InlineContent",
            "CodeBlock",
            "Badge",
            "KeyFigure",
            "EmptyState",
            "Icon",
            "Button",
            "TextField",
            "TextArea",
            "NumberField",
            "Select",
            "MultiSelect",
            "Switch",
            "Checkbox",
            "Slider",
            "DateTimeField",
            "Table",
            "RecordSummary",
            "DatasetChart",
            "Timeline",
            "Difference",
            "Progress",
            "Tabs",
            "Breadcrumb",
            "Menu",
            "Pager",
            "Link",
            "Alert",
            "Toast",
            "InlineMessage",
            "Skeleton",
            "Spinner",
            "StatusDot",
            "Dialog",
            "CapabilityCard",
            "ConsentForm",
            "SecurePrompt",
            "FilePicker",
            "ImageView",
            "AudioPlayer",
            "VideoPlayer",
            "FileDetails",
            "MediaEmbed",
            "CanvasStage",
            "DiagramView",
            "AnnotationLayer",
            "TerminalView",
            "NativePortal",
        ];
        assert_eq!(EXPECTED.len(), 62);
        assert_eq!(expected_names.len(), 62);
        for (primitive, name) in Primitive::ALL.into_iter().zip(expected_names) {
            assert_eq!(primitive.as_str(), name);
        }
    }

    #[test]
    fn every_record_is_complete_and_request_only() {
        let catalog = Catalog::v1();
        catalog.validate().expect("catalog is valid");
        for record in &catalog.records {
            assert!(!record.id.mints_capability());
            assert!(matches!(
                record.accessibility_label_strategy.to_lowercase(),
                value if value.contains("require") || value.contains("name")
            ));
            assert!(!record.states.is_empty());
        }
    }

    #[test]
    fn rejects_reordered_or_incomplete_catalogs() {
        let mut catalog = Catalog::v1();
        assert!(catalog.validate().is_ok());
        catalog.records.swap(0, 1);
        assert_eq!(catalog.validate(), Err(CatalogError::PrimitiveOrder));
        catalog = Catalog::v1();
        catalog.records.pop();
        assert_eq!(catalog.validate(), Err(CatalogError::PrimitiveCount));
    }
}
