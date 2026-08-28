//! Semantic component identities and the retained scene graph.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Maximum number of children a semantic node may contain.
pub const MAX_CHILDREN: usize = 64;
/// Maximum depth of a semantic scene.
pub const MAX_DEPTH: usize = 32;
/// Maximum number of nodes in one scene.
pub const MAX_NODES: usize = 512;
/// Maximum number of properties on a node.
pub const MAX_PROPS: usize = 32;
/// Maximum UTF-8 bytes in all properties of one node.
pub const MAX_PROP_BYTES: usize = 4096;
/// Maximum UTF-8 bytes in one accessibility string or semantic property key.
pub const MAX_SEMANTIC_TEXT_BYTES: usize = 512;
/// Union of all semantic state bits accepted by the catalog.
pub const KNOWN_STATE_BITS: u16 = (1 << 12) - 1;

/// The finite semantic catalog shipped by the reference shell.
///
/// These are semantic names, not renderer widgets.  In particular, a native
/// portal describes an external surface; it is never treated as an A2UI
/// component or a source of authority.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum ComponentKind {
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
    /// Action row.
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
    /// Source/code text.
    CodeBlock,
    /// Compact label.
    Badge,
    /// Prominent numeric fact.
    KeyFigure,
    /// Empty-result explanation.
    EmptyState,
    /// A named icon.
    Icon,
    /// An action control.  Icon-only controls use its icon slot.
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
    /// Date/time entry.
    DateTimeField,
    /// Tabular data.
    Table,
    /// Concise record summary.
    RecordSummary,
    /// Data chart with a textual equivalent.
    DatasetChart,
    /// Ordered events.
    Timeline,
    /// Before/after difference.
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
    /// Non-blocking alert.
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
    /// Governed file selection request.
    FilePicker,
    /// Image media.
    ImageView,
    /// Audio media.
    AudioPlayer,
    /// Video media.
    VideoPlayer,
    /// File metadata.
    FileDetails,
    /// Embedded media reference.
    MediaEmbed,
    /// Free-form drawing stage.
    CanvasStage,
    /// Diagram surface.
    DiagramView,
    /// Annotation layer.
    AnnotationLayer,
    /// Bounded terminal output.
    TerminalView,
    /// External application surface state.
    NativePortal,
}

impl ComponentKind {
    /// Every catalog kind in stable order.
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

    /// Return the catalog family used by the Theme Lab.
    pub const fn family(self) -> &'static str {
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
            | Self::Divider => "Structure",
            Self::Heading
            | Self::Text
            | Self::InlineContent
            | Self::CodeBlock
            | Self::Badge
            | Self::KeyFigure
            | Self::EmptyState
            | Self::Icon => "Content",
            Self::Button
            | Self::TextField
            | Self::TextArea
            | Self::NumberField
            | Self::Select
            | Self::MultiSelect
            | Self::Switch
            | Self::Checkbox
            | Self::Slider
            | Self::DateTimeField => "Input",
            Self::Table
            | Self::RecordSummary
            | Self::DatasetChart
            | Self::Timeline
            | Self::Difference
            | Self::Progress => "Data",
            Self::Tabs | Self::Breadcrumb | Self::Menu | Self::Pager | Self::Link => "Navigation",
            Self::Alert
            | Self::Toast
            | Self::InlineMessage
            | Self::Skeleton
            | Self::Spinner
            | Self::StatusDot
            | Self::Dialog => "Feedback",
            Self::CapabilityCard | Self::ConsentForm | Self::SecurePrompt | Self::FilePicker => {
                "Governed"
            }
            Self::ImageView
            | Self::AudioPlayer
            | Self::VideoPlayer
            | Self::FileDetails
            | Self::MediaEmbed => "Media",
            Self::CanvasStage | Self::DiagramView | Self::AnnotationLayer => "Canvas",
            Self::TerminalView => "Terminal",
            Self::NativePortal => "Native",
        }
    }
}

const INTERACTION: u16 = StateSet::HOVER | StateSet::PRESSED | StateSet::FOCUS | StateSet::DISABLED;
const SELECTABLE: u16 = INTERACTION | StateSet::SELECTED;
const STATUS: u16 = StateSet::LOADING
    | StateSet::ERROR
    | StateSet::EMPTY
    | StateSet::SUCCESS
    | StateSet::WARNING
    | StateSet::DANGER;
const RUNTIME_ONLY: u16 = INTERACTION | STATUS;
const GENERAL_PROPS: &[&str] = &[
    "title",
    "label",
    "name",
    "subtitle",
    "placeholder",
    "status",
    "value",
    "source",
    "text",
    "language",
    "alt",
    "duration",
    "action",
    "min",
    "max",
    "step",
    "options",
    "columns",
    "uri",
    "mime",
    "size",
    "modified",
    "progress",
    "collapse",
    "gap",
    "role",
];
const NO_CHILDREN: usize = 0;

/// Stable semantic identity derived from an activity-local key.
#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct NodeId(u64);

impl NodeId {
    /// Derive an identity without depending on insertion order.
    pub fn from_key(key: &str) -> Self {
        let digest = blake3::hash(key.as_bytes());
        let mut bytes = [0_u8; 8];
        bytes.copy_from_slice(&digest.as_bytes()[..8]);
        Self(u64::from_le_bytes(bytes))
    }

    /// Expose the compact value for display-list and snapshot serializers.
    pub const fn value(self) -> u64 {
        self.0
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "n{:016x}", self.0)
    }
}

/// A bounded scalar property value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PropValue {
    /// UTF-8 text, bounded by the containing property map.
    Text(String),
    /// Numeric value.
    Number(f64),
    /// Boolean value.
    Bool(bool),
    /// Semantic token reference.
    Token(String),
}

impl PropValue {
    fn byte_len(&self) -> usize {
        match self {
            Self::Text(value) | Self::Token(value) => value.len(),
            Self::Number(_) | Self::Bool(_) => 0,
        }
    }
}

/// Bounded node properties with deterministic key order.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(transparent)]
pub struct BoundedProps(BTreeMap<String, PropValue>);

impl BoundedProps {
    /// Build a property set while enforcing count and byte limits.
    pub fn new(values: BTreeMap<String, PropValue>) -> Result<Self, PropsError> {
        if values.len() > MAX_PROPS {
            return Err(PropsError::TooMany);
        }
        let bytes = values
            .iter()
            .map(|(key, value)| key.len() + value.byte_len())
            .sum::<usize>();
        if bytes > MAX_PROP_BYTES {
            return Err(PropsError::TooLarge);
        }
        Ok(Self(values))
    }

    /// Return an empty property set.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Add or replace one property, preserving the bounds.
    pub fn with(mut self, key: impl Into<String>, value: PropValue) -> Result<Self, PropsError> {
        self.0.insert(key.into(), value);
        Self::new(self.0)
    }

    /// Read a property.
    pub fn get(&self, key: &str) -> Option<&PropValue> {
        self.0.get(key)
    }

    /// Iterate in stable order.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &PropValue)> {
        self.0.iter()
    }
}

/// Property validation failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PropsError {
    /// More than [`MAX_PROPS`] keys were supplied.
    TooMany,
    /// Combined key and text bytes exceed [`MAX_PROP_BYTES`].
    TooLarge,
}

impl fmt::Display for PropsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooMany => write!(f, "component has too many properties"),
            Self::TooLarge => write!(f, "component properties exceed byte limit"),
        }
    }
}

impl std::error::Error for PropsError {}

/// Interaction and semantic state flags.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StateSet(u16);

impl StateSet {
    /// Pointer is over the component.
    pub const HOVER: u16 = 1 << 0;
    /// Pointer or keyboard activation is in progress.
    pub const PRESSED: u16 = 1 << 1;
    /// Component owns keyboard focus.
    pub const FOCUS: u16 = 1 << 2;
    /// Component cannot be activated.
    pub const DISABLED: u16 = 1 << 3;
    /// Component is selected.
    pub const SELECTED: u16 = 1 << 4;
    /// Component is waiting for work.
    pub const LOADING: u16 = 1 << 5;
    /// Component reports an error.
    pub const ERROR: u16 = 1 << 6;
    /// Component has no content.
    pub const EMPTY: u16 = 1 << 7;
    /// Operation succeeded.
    pub const SUCCESS: u16 = 1 << 8;
    /// Non-fatal warning.
    pub const WARNING: u16 = 1 << 9;
    /// Dangerous or destructive state.
    pub const DANGER: u16 = 1 << 10;
    /// Reserved validation bit, intentionally outside every component contract.
    pub const VALIDATION_ONLY: u16 = 1 << 11;

    /// Return an empty state set.
    pub const fn empty() -> Self {
        Self(0)
    }

    /// Set or clear one flag.
    pub const fn with(self, flag: u16, enabled: bool) -> Self {
        if enabled {
            Self(self.0 | flag)
        } else {
            Self(self.0 & !flag)
        }
    }

    /// Test one flag.
    pub const fn contains(self, flag: u16) -> bool {
        self.0 & flag != 0
    }

    /// Compact serialized value.
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// Construct from serialized bits, rejecting undefined flags.
    pub const fn from_bits(bits: u16) -> Result<Self, SceneError> {
        if bits & !KNOWN_STATE_BITS != 0 {
            Err(SceneError::InvalidState)
        } else {
            Ok(Self(bits))
        }
    }
}

impl TryFrom<u16> for StateSet {
    type Error = SceneError;

    fn try_from(bits: u16) -> Result<Self, Self::Error> {
        Self::from_bits(bits)
    }
}

impl Serialize for StateSet {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for StateSet {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let bits = u16::deserialize(deserializer)?;
        Self::from_bits(bits).map_err(serde::de::Error::custom)
    }
}

/// Accessible semantic metadata kept beside the rendered node.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Accessibility {
    /// Semantic role exposed to an accessibility adapter.
    pub role: String,
    /// Human-readable name.
    pub name: String,
    /// Optional description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl Accessibility {
    /// Construct a named semantic role.
    pub fn named(role: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            name: name.into(),
            description: None,
        }
    }
}

impl<'de> Deserialize<'de> for Accessibility {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            role: String,
            name: String,
            description: Option<String>,
        }
        let raw = Raw::deserialize(deserializer)?;
        if raw.role.is_empty() || raw.role.len() > MAX_SEMANTIC_TEXT_BYTES {
            return Err(serde::de::Error::custom("invalid accessibility role"));
        }
        if raw.name.is_empty() || raw.name.len() > MAX_SEMANTIC_TEXT_BYTES {
            return Err(serde::de::Error::custom("invalid accessibility name"));
        }
        if raw
            .description
            .as_ref()
            .is_some_and(|text| text.len() > MAX_SEMANTIC_TEXT_BYTES)
        {
            return Err(serde::de::Error::custom(
                "invalid accessibility description",
            ));
        }
        Ok(Self {
            role: raw.role,
            name: raw.name,
            description: raw.description,
        })
    }
}

impl<'de> Deserialize<'de> for BoundedProps {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let values = BTreeMap::<String, PropValue>::deserialize(deserializer)?;
        Self::new(values).map_err(serde::de::Error::custom)
    }
}

/// A retained semantic node.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SemanticNode {
    /// Stable identity used by reconciliation and focus.
    pub id: NodeId,
    /// Semantic component kind.
    pub kind: ComponentKind,
    /// Bounded semantic properties.
    #[serde(default)]
    pub props: BoundedProps,
    /// Interaction and status state.
    #[serde(default)]
    pub state: StateSet,
    /// Accessible role/name/state metadata.
    #[serde(default)]
    pub accessibility: Accessibility,
    /// Ordered child nodes.
    #[serde(default)]
    pub children: Vec<Self>,
}

impl<'de> Deserialize<'de> for SemanticNode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawNode {
            id: NodeId,
            kind: ComponentKind,
            #[serde(default)]
            props: BoundedProps,
            #[serde(default)]
            state: StateSet,
            #[serde(default)]
            accessibility: Accessibility,
            #[serde(default)]
            children: Vec<RawNode>,
        }

        fn validate(
            node: &RawNode,
            depth: usize,
            parent: Option<NodeId>,
            seen: &mut BTreeSet<NodeId>,
        ) -> Result<usize, SceneError> {
            if depth > MAX_DEPTH {
                return Err(SceneError::TooDeep(node.id));
            }
            if parent == Some(node.id) {
                return Err(SceneError::SelfParent(node.id));
            }
            if !seen.insert(node.id) {
                return Err(SceneError::DuplicateNode(node.id));
            }
            let contract = node.kind.contract();
            if node.children.len() > contract.max_children {
                return Err(SceneError::TooManyChildren(node.id));
            }
            if node.state.bits() & !contract.allowed_state != 0 {
                return Err(SceneError::UnsupportedState(node.id, node.kind));
            }
            for key in contract.required_props {
                if node.props.get(key).is_none() {
                    return Err(SceneError::MissingProperty(node.id, node.kind, key));
                }
            }
            for key in node.props.iter().map(|(key, _)| key.as_str()) {
                if !contract.allowed_props.contains(&key) {
                    return Err(SceneError::UnknownProperty(
                        node.id,
                        node.kind,
                        key.to_owned(),
                    ));
                }
            }
            let mut total = 1_usize;
            for child in &node.children {
                total = total
                    .checked_add(validate(child, depth + 1, Some(node.id), seen)?)
                    .ok_or(SceneError::TooManyNodes)?;
                if total > MAX_NODES {
                    return Err(SceneError::TooManyNodes);
                }
            }
            Ok(total)
        }

        fn expand(node: RawNode) -> SemanticNode {
            SemanticNode {
                id: node.id,
                kind: node.kind,
                props: node.props,
                state: node.state,
                accessibility: node.accessibility,
                children: node.children.into_iter().map(expand).collect(),
            }
        }

        let raw = RawNode::deserialize(deserializer)?;
        let mut seen = BTreeSet::new();
        validate(&raw, 0, None, &mut seen).map_err(serde::de::Error::custom)?;
        Ok(expand(raw))
    }
}

/// Typed construction/validation contract for one catalog identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ComponentContract {
    /// Stable family name.
    pub family: &'static str,
    /// Properties that must be present for a meaningful semantic node.
    pub required_props: &'static [&'static str],
    /// Properties accepted by this identity.
    pub allowed_props: &'static [&'static str],
    /// State flags accepted by this identity.
    pub allowed_state: u16,
    /// Maximum children accepted by this identity.
    pub max_children: usize,
}

impl SemanticNode {
    /// Create a node with a stable key and no children.
    pub fn new(key: &str, kind: ComponentKind, name: &str) -> Self {
        Self {
            id: NodeId::from_key(key),
            kind,
            props: BoundedProps::default(),
            state: StateSet::empty(),
            accessibility: Accessibility::named(kind.family(), name),
            children: Vec::new(),
        }
    }

    /// Attach a child, enforcing the local count bound.
    pub fn push(&mut self, child: Self) -> Result<(), SceneError> {
        if self.children.len() >= MAX_CHILDREN {
            return Err(SceneError::TooManyChildren(self.id));
        }
        self.children.push(child);
        Ok(())
    }

    /// Validate depth, fan-out, and total node count.
    pub fn validate(&self) -> Result<usize, SceneError> {
        let mut seen = BTreeSet::new();
        self.validate_at(0, &mut seen)
    }

    fn validate_at(&self, depth: usize, seen: &mut BTreeSet<NodeId>) -> Result<usize, SceneError> {
        if depth > MAX_DEPTH {
            return Err(SceneError::TooDeep(self.id));
        }
        if !seen.insert(self.id) {
            return Err(SceneError::DuplicateNode(self.id));
        }
        let contract = self.kind.contract();
        if self.children.len() > contract.max_children {
            return Err(SceneError::TooManyChildren(self.id));
        }
        if self.state.bits() & !contract.allowed_state != 0 {
            return Err(SceneError::UnsupportedState(self.id, self.kind));
        }
        for key in contract.required_props {
            if self.props.get(key).is_none() {
                return Err(SceneError::MissingProperty(self.id, self.kind, key));
            }
        }
        for key in self.props.iter().map(|(key, _)| key.as_str()) {
            if !contract.allowed_props.contains(&key) {
                return Err(SceneError::UnknownProperty(
                    self.id,
                    self.kind,
                    key.to_owned(),
                ));
            }
        }
        let mut total = 1_usize;
        for child in &self.children {
            total = total
                .checked_add(child.validate_at(depth + 1, seen)?)
                .ok_or(SceneError::TooManyNodes)?;
            if total > MAX_NODES {
                return Err(SceneError::TooManyNodes);
            }
            if child.id == self.id {
                return Err(SceneError::SelfParent(self.id));
            }
        }
        Ok(total)
    }

    /// Walk this node and all descendants in reading order.
    pub fn walk<'a>(&'a self, output: &mut Vec<&'a Self>) {
        output.push(self);
        for child in &self.children {
            child.walk(output);
        }
    }
}

impl ComponentKind {
    /// Return the typed semantic contract for this identity.
    pub fn contract(self) -> ComponentContract {
        use ComponentKind as K;
        let (required, state, children) = match self {
            K::Heading => (&["text"][..], INTERACTION, NO_CHILDREN),
            K::Text | K::InlineContent => (&["text"][..], INTERACTION, NO_CHILDREN),
            K::CodeBlock => (&["text", "language"][..], INTERACTION, NO_CHILDREN),
            K::Button => (&["label", "action"][..], SELECTABLE | STATUS, NO_CHILDREN),
            K::TextField | K::TextArea | K::NumberField | K::DateTimeField => {
                (&["value"][..], SELECTABLE | STATUS, NO_CHILDREN)
            }
            K::Select | K::MultiSelect => {
                (&["value", "options"][..], SELECTABLE | STATUS, NO_CHILDREN)
            }
            K::Switch | K::Checkbox => (&["value"][..], SELECTABLE | STATUS, NO_CHILDREN),
            K::Slider => (
                &["value", "min", "max", "step"][..],
                SELECTABLE | STATUS,
                NO_CHILDREN,
            ),
            K::Table => (&["columns"][..], SELECTABLE | STATUS, MAX_CHILDREN),
            K::DatasetChart => (&["source"][..], STATUS, NO_CHILDREN),
            K::Progress => (&["value", "min", "max"][..], STATUS, NO_CHILDREN),
            K::Tabs => (&["options"][..], SELECTABLE, MAX_CHILDREN),
            K::Link => (&["label", "uri"][..], INTERACTION, NO_CHILDREN),
            K::ImageView => (&["source", "alt"][..], STATUS, NO_CHILDREN),
            K::AudioPlayer | K::VideoPlayer => (&["source", "duration"][..], STATUS, NO_CHILDREN),
            K::FileDetails => (&["name", "size"][..], STATUS, NO_CHILDREN),
            K::NativePortal => (&["status"][..], STATUS | StateSet::EMPTY, NO_CHILDREN),
            K::Repeater | K::Grid | K::Stack | K::Split => (&[][..], STATUS, MAX_CHILDREN),
            _ => (&[][..], RUNTIME_ONLY, MAX_CHILDREN),
        };
        ComponentContract {
            family: self.family(),
            required_props: required,
            allowed_props: GENERAL_PROPS,
            allowed_state: state,
            max_children: children,
        }
    }
}

/// Scene validation failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SceneError {
    /// A node exceeded the maximum nesting depth.
    TooDeep(NodeId),
    /// A node exceeded the maximum child count.
    TooManyChildren(NodeId),
    /// A scene exceeded the maximum total node count.
    TooManyNodes,
    /// The same node identity occurred twice.
    DuplicateNode(NodeId),
    /// A parent directly contains itself.
    SelfParent(NodeId),
    /// Undefined state bits were supplied.
    InvalidState,
    /// A state bit is not valid for the component identity.
    UnsupportedState(NodeId, ComponentKind),
    /// A required family property is absent.
    MissingProperty(NodeId, ComponentKind, &'static str),
    /// A property is not accepted by the component identity.
    UnknownProperty(NodeId, ComponentKind, String),
    /// Surface identity was empty or oversized.
    InvalidSurfaceId,
}

impl fmt::Display for SceneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooDeep(id) => write!(f, "scene node {id} is too deep"),
            Self::TooManyChildren(id) => write!(f, "scene node {id} has too many children"),
            Self::TooManyNodes => write!(f, "scene has too many nodes"),
            Self::DuplicateNode(id) => write!(f, "duplicate semantic node identity {id}"),
            Self::SelfParent(id) => write!(f, "semantic node {id} is its own parent"),
            Self::InvalidState => write!(f, "undefined semantic state bit"),
            Self::UnsupportedState(id, kind) => {
                write!(f, "unsupported state for {kind:?} node {id}")
            }
            Self::MissingProperty(id, kind, key) => {
                write!(f, "{kind:?} node {id} is missing property `{key}`")
            }
            Self::UnknownProperty(id, kind, key) => {
                write!(f, "{kind:?} node {id} does not accept property `{key}`")
            }
            Self::InvalidSurfaceId => write!(f, "invalid surface identity"),
        }
    }
}

impl std::error::Error for SceneError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_exactly_sixty_two_and_families_are_complete() {
        assert_eq!(ComponentKind::ALL.len(), 62);
        let mut names = std::collections::BTreeSet::new();
        for kind in ComponentKind::ALL {
            assert!(names.insert(format!("{kind:?}")));
        }
        assert_eq!(names.len(), 62);
        assert_eq!(
            ComponentKind::ALL
                .iter()
                .filter(|kind| kind.family() == "Structure")
                .count(),
            12
        );
    }

    #[test]
    fn node_ids_are_stable_and_property_bounds_hold() {
        assert_eq!(
            NodeId::from_key("activity/cats"),
            NodeId::from_key("activity/cats")
        );
        let mut values = BTreeMap::new();
        for i in 0..=MAX_PROPS {
            values.insert(i.to_string(), PropValue::Bool(true));
        }
        assert_eq!(BoundedProps::new(values), Err(PropsError::TooMany));
    }

    #[test]
    fn duplicate_ids_are_rejected_before_reconciliation() {
        let mut root = SemanticNode::new("root", ComponentKind::Region, "Root");
        root.push(SemanticNode::new("shared", ComponentKind::Group, "One"))
            .expect("one");
        root.push(SemanticNode::new("shared", ComponentKind::Group, "Two"))
            .expect("two");
        assert!(matches!(root.validate(), Err(SceneError::DuplicateNode(_))));
    }

    #[test]
    fn every_identity_rejects_malformed_state() {
        for kind in ComponentKind::ALL {
            let mut node = SemanticNode::new("contract", kind, "Contract");
            let forbidden = (0..12_u32)
                .map(|bit| 1 << bit)
                .find(|flag| kind.contract().allowed_state & flag == 0)
                .expect("every contract excludes at least one state");
            node.state = StateSet::from_bits(forbidden).expect("valid catalog bit");
            assert!(matches!(
                node.validate(),
                Err(SceneError::UnsupportedState(_, _))
            ));
        }
    }

    #[test]
    fn serialization_validates_semantic_bounds() {
        let mut root = SemanticNode::new("root", ComponentKind::Region, "Root");
        root.push(SemanticNode::new("dupe", ComponentKind::Group, "One"))
            .expect("child");
        root.push(SemanticNode::new("dupe", ComponentKind::Group, "Two"))
            .expect("child");
        let bytes = serde_json::to_vec(&root).expect("serialize raw semantic shape");
        assert!(serde_json::from_slice::<SemanticNode>(&bytes).is_err());
    }
}
