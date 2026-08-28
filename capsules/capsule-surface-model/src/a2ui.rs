//! Declared-loss A2UI import/export boundary.

use crate::activity::{OpaqueOwnerRef, OpaquePrincipalRef};
use crate::canonical::{
    CanonicalJson, MAX_CANONICAL_DOCUMENT_BYTES, canonical_bytes, canonical_string,
};
use crate::components::{ComponentKind, PropValue, SemanticNode};
use crate::recipe::Recipe;
use crate::store::{MAX_PATCH_OPS, Patch, PatchError, PatchOp};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const AUTHORITY_LABELS: &[&str] = &[
    "owner",
    "principal",
    "actor",
    "authority",
    "grant",
    "capability",
    "capabilities",
    "scope",
    "scopes",
    "token",
    "cookie",
    "credential",
    "secret",
    "handle",
    "fd",
    "socket",
    "pid",
    "argv",
    "env",
    "path",
    "home",
    "xdg",
];

const MAX_A2UI_DATA_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum A2uiVersion {
    V0_9,
    V1_0,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateSurfaceMessage {
    pub version: A2uiVersion,
    pub surface_id: String,
    pub recipe_id: String,
    pub root: SemanticNode,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateComponentsMessage {
    pub version: A2uiVersion,
    pub surface_id: String,
    pub recipe_id: String,
    pub root: SemanticNode,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateDataModelMessage {
    pub version: A2uiVersion,
    pub surface_id: String,
    pub recipe_id: String,
    pub data: CanonicalJson,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteSurfaceMessage {
    pub version: A2uiVersion,
    pub surface_id: String,
    pub recipe_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "message", rename_all = "kebab-case", deny_unknown_fields)]
pub enum StorageImport {
    CreateSurface(CreateSurfaceMessage),
    UpdateComponents(UpdateComponentsMessage),
    UpdateDataModel(UpdateDataModelMessage),
    DeleteSurface(DeleteSurfaceMessage),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "message", rename_all = "kebab-case", deny_unknown_fields)]
#[allow(clippy::large_enum_variant)]
pub enum NonStorageCall {
    Renderer {
        frame: CanonicalJson,
    },
    AgentFunctionCall {
        function: String,
        arguments: CanonicalJson,
    },
}

#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum ImportedDocument {
    Recipe(Recipe),
    Patch(Patch),
    EphemeralDelete,
}

pub struct A2uiAdapter;

impl A2uiAdapter {
    pub fn import_create_surface(
        message: &CreateSurfaceMessage,
        owner_ref: OpaqueOwnerRef,
        _acting_principal: OpaquePrincipalRef,
        theme_id: String,
    ) -> Result<Recipe, A2uiError> {
        Self::validate_version(message.version)?;
        Self::valid_id(&message.surface_id, "surface id")?;
        Self::valid_id(&message.recipe_id, "recipe id")?;
        Self::bounded_value(&message.root)?;
        message
            .root
            .validate()
            .map_err(|_| A2uiError::UnsupportedCatalog)?;
        Recipe::new(
            owner_ref,
            message.recipe_id.clone(),
            theme_id,
            message.root.clone(),
        )
        .map_err(A2uiError::Recipe)
    }

    pub fn import_update_components(
        message: &UpdateComponentsMessage,
        owner_ref: OpaqueOwnerRef,
        actor: OpaquePrincipalRef,
        reviewer: OpaquePrincipalRef,
        receipt: String,
        base: &Recipe,
    ) -> Result<Patch, A2uiError> {
        Self::validate_version(message.version)?;
        Self::same_target(message, base)?;
        Self::bounded_value(&message.root)?;
        message
            .root
            .validate()
            .map_err(|_| A2uiError::UnsupportedCatalog)?;
        Patch::new(
            owner_ref,
            actor,
            reviewer,
            receipt,
            format!("a2ui:{}:components", message.surface_id),
            base.recipe_id.clone(),
            base.revision,
            base.digest.clone(),
            format!(
                "A2UI {} component update",
                match message.version {
                    A2uiVersion::V0_9 => "v0.9",
                    A2uiVersion::V1_0 => "v1.0",
                }
            ),
            vec![PatchOp::ReplaceRoot {
                root: message.root.clone(),
            }],
        )
        .map_err(A2uiError::Patch)
    }

    pub fn import_update_data_model(
        message: &UpdateDataModelMessage,
        owner_ref: OpaqueOwnerRef,
        actor: OpaquePrincipalRef,
        reviewer: OpaquePrincipalRef,
        receipt: String,
        base: &Recipe,
    ) -> Result<Patch, A2uiError> {
        Self::validate_version(message.version)?;
        Self::same_target(message, base)?;
        Self::bounded_value_at(&message.data, MAX_A2UI_DATA_BYTES)?;
        Self::reject_authority_value(&message.data)?;
        let entries = match &message.data {
            CanonicalJson::Object(entries) => entries,
            _ => return Err(A2uiError::UnsupportedCatalog),
        };
        if entries.is_empty() || entries.len() > crate::recipe::MAX_RECIPE_METADATA {
            return Err(A2uiError::UnsupportedCatalog);
        }
        let mut operations = Vec::new();
        for (key, value) in entries {
            Self::reject_authority_label(key)?;
            let stored_key = format!("a2ui/data:{key}");
            let encoded = canonical_string(value).map_err(|_| A2uiError::UnsupportedCatalog)?;
            operations.push(PatchOp::SetRecipeMetadata {
                key: stored_key,
                value: PropValue::Text(encoded),
            });
        }
        Patch::new(
            owner_ref,
            actor,
            reviewer,
            receipt,
            format!("a2ui:{}:data", message.surface_id),
            base.recipe_id.clone(),
            base.revision,
            base.digest.clone(),
            "A2UI data model update",
            operations,
        )
        .map_err(A2uiError::Patch)
    }

    pub fn import_delete_surface(
        message: &DeleteSurfaceMessage,
    ) -> Result<ImportedDocument, A2uiError> {
        Self::validate_version(message.version)?;
        Self::valid_id(&message.surface_id, "surface id")?;
        Self::valid_id(&message.recipe_id, "recipe id")?;
        Ok(ImportedDocument::EphemeralDelete)
    }

    pub fn reject_non_storage(call: &NonStorageCall) -> Result<ImportedDocument, A2uiError> {
        match call {
            NonStorageCall::Renderer { .. } | NonStorageCall::AgentFunctionCall { .. } => {
                Err(A2uiError::NonStorage)
            }
        }
    }

    fn validate_version(version: A2uiVersion) -> Result<(), A2uiError> {
        match version {
            A2uiVersion::V0_9 | A2uiVersion::V1_0 => Ok(()),
        }
    }

    fn valid_id(value: &str, label: &str) -> Result<(), A2uiError> {
        if value.is_empty()
            || value.len() > 256
            || value.contains("..")
            || !value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'))
        {
            return Err(A2uiError::UnsupportedCatalog);
        }
        let _ = label;
        Ok(())
    }

    fn bounded_value<T: serde::Serialize>(value: &T) -> Result<(), A2uiError> {
        Self::bounded_value_at(value, MAX_CANONICAL_DOCUMENT_BYTES)
    }

    fn bounded_value_at<T: serde::Serialize>(value: &T, maximum: usize) -> Result<(), A2uiError> {
        let size = canonical_bytes(value)
            .map_err(|_| A2uiError::UnsupportedCatalog)?
            .len();
        if size > maximum {
            Err(A2uiError::Oversized)
        } else {
            Ok(())
        }
    }

    fn same_target<M>(message: &M, base: &Recipe) -> Result<(), A2uiError>
    where
        M: A2uiTarget,
    {
        Self::valid_id(message.surface_id(), "surface id")?;
        if message.recipe_id() != base.recipe_id {
            return Err(A2uiError::UnsupportedCatalog);
        }
        Ok(())
    }

    fn reject_authority_label(key: &str) -> Result<(), A2uiError> {
        let normalized = key.to_ascii_lowercase().replace(['-', '_', ' '], "");
        if AUTHORITY_LABELS.contains(&normalized.as_str())
            || normalized.contains("path")
            || normalized.contains("home")
            || normalized.contains("xdg")
        {
            return Err(A2uiError::AuthorityField);
        }
        Ok(())
    }

    fn reject_authority_value(value: &CanonicalJson) -> Result<(), A2uiError> {
        match value {
            CanonicalJson::Array(values) => {
                for value in values {
                    Self::reject_authority_value(value)?;
                }
                Ok(())
            }
            CanonicalJson::Object(values) => {
                for (key, value) in values {
                    Self::reject_authority_label(key)?;
                    Self::reject_authority_value(value)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

trait A2uiTarget {
    fn surface_id(&self) -> &str;
    fn recipe_id(&self) -> &str;
}

macro_rules! impl_a2ui_target {
    ($($kind:ty),+) => {
        $(impl A2uiTarget for $kind {
            fn surface_id(&self) -> &str { &self.surface_id }
            fn recipe_id(&self) -> &str { &self.recipe_id }
        })+
    };
}

impl_a2ui_target!(UpdateComponentsMessage, UpdateDataModelMessage);

#[derive(Clone, Debug)]
pub struct ExportCatalog {
    supported: BTreeSet<ComponentKind>,
}

impl ExportCatalog {
    pub fn minimal() -> Self {
        Self {
            supported: [
                ComponentKind::Region,
                ComponentKind::Text,
                ComponentKind::Button,
            ]
            .into_iter()
            .collect(),
        }
    }

    pub fn supports(&self, kind: ComponentKind) -> bool {
        self.supported.contains(&kind)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ExportProjection {
    pub version: A2uiVersion,
    pub surface_id: String,
    pub recipe_id: String,
    pub recipe_revision: u64,
    pub lossy: bool,
    pub degraded_nodes: usize,
    pub root: CanonicalJson,
}

pub fn export_projection(
    recipe: &Recipe,
    surface_id: &str,
    version: A2uiVersion,
    catalog: &ExportCatalog,
) -> Result<ExportProjection, A2uiError> {
    if catalog.supported.is_empty() || surface_id.is_empty() || surface_id.len() > 256 {
        return Err(A2uiError::UnsupportedCatalog);
    }
    let mut degraded = 0_usize;
    let root = project_node(&recipe.root, catalog, &mut degraded)?;
    Ok(ExportProjection {
        version,
        surface_id: surface_id.to_owned(),
        recipe_id: recipe.recipe_id.clone(),
        recipe_revision: recipe.revision,
        lossy: degraded > 0,
        degraded_nodes: degraded,
        root,
    })
}

fn project_node(
    node: &SemanticNode,
    catalog: &ExportCatalog,
    degraded: &mut usize,
) -> Result<CanonicalJson, A2uiError> {
    if !catalog.supports(node.kind) {
        *degraded = degraded.saturating_add(1);
        let mut object = BTreeMap::new();
        object.insert(
            "kind".to_owned(),
            CanonicalJson::String("unsupported".to_owned()),
        );
        object.insert(
            "reason".to_owned(),
            CanonicalJson::String("catalog-subset".to_owned()),
        );
        return Ok(CanonicalJson::Object(object));
    }
    let mut object = BTreeMap::new();
    let mut projected = BTreeSet::new();
    object.insert(
        "kind".to_owned(),
        CanonicalJson::String(component_label(node.kind)),
    );
    if let Some(accessible_name) = node.props.get("label").or(node.props.get("text"))
        && let Some(text) = prop_text(accessible_name)
    {
        object.insert("text".to_owned(), CanonicalJson::String(text));
        if node.props.get("label").is_some() {
            projected.insert("label");
        } else {
            projected.insert("text");
        }
    }
    match node.props.get("action") {
        Some(value) => {
            projected.insert("action");
            match prop_text(value) {
                Some(text) => {
                    object.insert("action".to_owned(), CanonicalJson::String(text));
                }
                None => {
                    *degraded = degraded.saturating_add(1);
                }
            }
        }
        None if node.kind == ComponentKind::Button => {
            *degraded = degraded.saturating_add(1);
        }
        None => {}
    }
    for (key, _) in node.props.iter() {
        if !projected.contains(key.as_str()) {
            *degraded = degraded.saturating_add(1);
        }
    }
    let children = node
        .children
        .iter()
        .map(|child| project_node(child, catalog, degraded))
        .collect::<Result<Vec<_>, _>>()?;
    if !children.is_empty() {
        object.insert("children".to_owned(), CanonicalJson::Array(children));
    }
    Ok(CanonicalJson::Object(object))
}

fn prop_text(value: &PropValue) -> Option<String> {
    match value {
        PropValue::Text(value) | PropValue::Token(value) => Some(value.clone()),
        _ => None,
    }
}

fn component_label(kind: ComponentKind) -> String {
    match kind {
        ComponentKind::Region => "region",
        ComponentKind::Text => "text",
        ComponentKind::Button => "button",
        _ => "unsupported",
    }
    .to_owned()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum A2uiError {
    UnsupportedCatalog,
    AuthorityField,
    NonStorage,
    Oversized,
    Patch(PatchError),
    Recipe(crate::recipe::RecipeValidationError),
}

impl std::fmt::Display for A2uiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedCatalog => f.write_str("A2UI catalog or document is unsupported"),
            Self::AuthorityField => f.write_str("A2UI input contains an authority-bearing field"),
            Self::NonStorage => f.write_str("renderer or function-call output is not storage"),
            Self::Oversized => f.write_str("A2UI document exceeds the bounded size"),
            Self::Patch(error) => error.fmt(f),
            Self::Recipe(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for A2uiError {}

impl From<crate::recipe::RecipeValidationError> for A2uiError {
    fn from(value: crate::recipe::RecipeValidationError) -> Self {
        Self::Recipe(value)
    }
}

const _: () = {
    assert!(MAX_PATCH_OPS >= 2);
};
