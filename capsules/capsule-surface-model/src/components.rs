//! Bounded semantic scene catalog shared by recipes and ephemeral surfaces.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const MAX_CHILDREN: usize = 64;
pub const MAX_DEPTH: usize = 32;
pub const MAX_NODES: usize = 512;
pub const MAX_PROPS: usize = 32;
pub const MAX_PROP_BYTES: usize = 4096;
pub const MAX_SEMANTIC_TEXT_BYTES: usize = 512;
pub const KNOWN_STATE_BITS: u16 = (1 << 12) - 1;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum ComponentKind {
    Region,
    Stack,
    Grid,
    Split,
    Sidebar,
    ActionBar,
    Card,
    Group,
    Collapse,
    Repeater,
    ScrollRegion,
    Divider,
    Heading,
    Text,
    InlineContent,
    CodeBlock,
    Badge,
    KeyFigure,
    EmptyState,
    Icon,
    Button,
    TextField,
    TextArea,
    NumberField,
    Select,
    MultiSelect,
    Switch,
    Checkbox,
    Slider,
    DateTimeField,
    Table,
    RecordSummary,
    DatasetChart,
    Timeline,
    Difference,
    Progress,
    Tabs,
    Breadcrumb,
    Menu,
    Pager,
    Link,
    Alert,
    Toast,
    InlineMessage,
    Skeleton,
    Spinner,
    StatusDot,
    Dialog,
    CapabilityCard,
    ConsentForm,
    SecurePrompt,
    FilePicker,
    ImageView,
    AudioPlayer,
    VideoPlayer,
    FileDetails,
    MediaEmbed,
    CanvasStage,
    DiagramView,
    AnnotationLayer,
    TerminalView,
    NativePortal,
}

impl ComponentKind {
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

#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct NodeId(u64);

impl NodeId {
    pub fn from_key(key: &str) -> Self {
        let digest = blake3::hash(key.as_bytes());
        let mut bytes = [0_u8; 8];
        bytes.copy_from_slice(&digest.as_bytes()[..8]);
        Self(u64::from_le_bytes(bytes))
    }

    pub const fn value(self) -> u64 {
        self.0
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "n{:016x}", self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PropValue {
    Text(String),
    Number(f64),
    Bool(bool),
    Token(String),
}

impl PropValue {
    pub fn byte_len(&self) -> usize {
        match self {
            Self::Text(value) | Self::Token(value) => value.len(),
            Self::Number(value) if value.is_finite() => 8,
            Self::Number(_) => usize::MAX,
            Self::Bool(_) => 1,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "BTreeMap<String, PropValue>")]
pub struct BoundedProps(BTreeMap<String, PropValue>);

impl TryFrom<BTreeMap<String, PropValue>> for BoundedProps {
    type Error = PropsError;

    fn try_from(values: BTreeMap<String, PropValue>) -> Result<Self, Self::Error> {
        Self::new(values)
    }
}

impl BoundedProps {
    pub fn new(values: BTreeMap<String, PropValue>) -> Result<Self, PropsError> {
        if values.len() > MAX_PROPS {
            return Err(PropsError::TooMany);
        }
        let bytes = values
            .iter()
            .map(|(key, value)| key.len().saturating_add(value.byte_len()))
            .sum::<usize>();
        if bytes > MAX_PROP_BYTES {
            return Err(PropsError::TooLarge);
        }
        Ok(Self(values))
    }

    pub fn empty() -> Self {
        Self::default()
    }

    pub fn with(mut self, key: impl Into<String>, value: PropValue) -> Result<Self, PropsError> {
        self.0.insert(key.into(), value);
        Self::new(self.0)
    }

    pub fn get(&self, key: &str) -> Option<&PropValue> {
        self.0.get(key)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &PropValue)> {
        self.0.iter()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PropsError {
    TooMany,
    TooLarge,
}

impl fmt::Display for PropsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooMany => f.write_str("component has too many properties"),
            Self::TooLarge => f.write_str("component properties exceed byte limit"),
        }
    }
}

impl std::error::Error for PropsError {}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StateSet(u16);

impl StateSet {
    pub const HOVER: u16 = 1 << 0;
    pub const PRESSED: u16 = 1 << 1;
    pub const FOCUS: u16 = 1 << 2;
    pub const DISABLED: u16 = 1 << 3;
    pub const SELECTED: u16 = 1 << 4;
    pub const LOADING: u16 = 1 << 5;
    pub const ERROR: u16 = 1 << 6;
    pub const EMPTY: u16 = 1 << 7;
    pub const SUCCESS: u16 = 1 << 8;
    pub const WARNING: u16 = 1 << 9;
    pub const DANGER: u16 = 1 << 10;
    pub const VALIDATION_ONLY: u16 = 1 << 11;

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn with(self, flag: u16, enabled: bool) -> Self {
        if enabled {
            Self(self.0 | flag)
        } else {
            Self(self.0 & !flag)
        }
    }

    pub const fn contains(self, flag: u16) -> bool {
        self.0 & flag != 0
    }

    pub const fn bits(self) -> u16 {
        self.0
    }

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

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Accessibility {
    pub role: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl Accessibility {
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
        if !bounded_text(&raw.role) || !bounded_text(&raw.name) {
            return Err(serde::de::Error::custom("invalid accessibility metadata"));
        }
        if raw
            .description
            .as_ref()
            .is_some_and(|text| !bounded_text(text))
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

fn bounded_text(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_SEMANTIC_TEXT_BYTES
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SemanticNode {
    pub id: NodeId,
    pub kind: ComponentKind,
    #[serde(default)]
    pub props: BoundedProps,
    #[serde(default)]
    pub state: StateSet,
    #[serde(default)]
    pub accessibility: Accessibility,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ComponentContract {
    pub family: &'static str,
    pub required_props: &'static [&'static str],
    pub allowed_props: &'static [&'static str],
    pub allowed_state: u16,
    pub max_children: usize,
}

impl SemanticNode {
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

    pub fn push(&mut self, child: Self) -> Result<(), SceneError> {
        if self.children.len() >= MAX_CHILDREN {
            return Err(SceneError::TooManyChildren(self.id));
        }
        self.children.push(child);
        Ok(())
    }

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

    pub fn find_mut(&mut self, target: NodeId) -> Option<&mut Self> {
        if self.id == target {
            return Some(self);
        }
        self.children
            .iter_mut()
            .find_map(|child| child.find_mut(target))
    }
}

impl ComponentKind {
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SceneError {
    TooDeep(NodeId),
    TooManyChildren(NodeId),
    TooManyNodes,
    DuplicateNode(NodeId),
    SelfParent(NodeId),
    InvalidState,
    UnsupportedState(NodeId, ComponentKind),
    MissingProperty(NodeId, ComponentKind, &'static str),
    UnknownProperty(NodeId, ComponentKind, String),
    InvalidSurfaceId,
}

impl fmt::Display for SceneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooDeep(id) => write!(f, "scene node {id} is too deep"),
            Self::TooManyChildren(id) => write!(f, "scene node {id} has too many children"),
            Self::TooManyNodes => f.write_str("scene has too many nodes"),
            Self::DuplicateNode(id) => write!(f, "duplicate semantic node identity {id}"),
            Self::SelfParent(id) => write!(f, "semantic node {id} is its own parent"),
            Self::InvalidState => f.write_str("undefined semantic state bit"),
            Self::UnsupportedState(id, kind) => {
                write!(f, "unsupported state for {kind:?} node {id}")
            }
            Self::MissingProperty(id, kind, key) => {
                write!(f, "{kind:?} node {id} is missing `{key}`")
            }
            Self::UnknownProperty(id, kind, key) => write!(f, "{kind:?} node {id} rejects `{key}`"),
            Self::InvalidSurfaceId => f.write_str("invalid surface identity"),
        }
    }
}

impl std::error::Error for SceneError {}
