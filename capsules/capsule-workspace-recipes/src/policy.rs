use capsule_surface_model::activity::{OpaqueOwnerRef, OpaquePrincipalRef};
use capsule_surface_model::canonical::{CanonicalJson, canonical_bytes};
use std::collections::BTreeMap;

const FORBIDDEN_KEYS: &[&str] = &[
    "access_token",
    "api_key",
    "argv",
    "aos.surface",
    "authorization",
    "bearer",
    "cookie",
    "credential",
    "current_surface",
    "dom",
    "env",
    "file_descriptor",
    "function_call",
    "grant_token",
    "handle",
    "home",
    "host_path",
    "html",
    "incarnation",
    "mount",
    "password",
    "path",
    "pid",
    "pixels",
    "renderer",
    "screenshot",
    "secret",
    "socket",
    "surface_id",
    "token",
    "xdg",
];

const FORBIDDEN_VALUES: &[&str] = &[
    "aos.surface@1",
    "bearer ",
    "file://",
    "function-call",
    "ghp_",
    "home://",
    "tmp://",
    "workspace://",
    "/users/",
    "$home",
    "-----begin",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GrantRecord {
    pub owner: OpaqueOwnerRef,
    pub authorized_actor: OpaquePrincipalRef,
    pub generation: u64,
    pub revoked: bool,
}

#[derive(Clone, Debug, Default)]
pub struct GrantDirectory {
    grants: BTreeMap<String, GrantRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GrantProof {
    pub grant_id: String,
    pub generation: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnerContext {
    pub owner: OpaqueOwnerRef,
    pub acting_principal: OpaquePrincipalRef,
    pub grant: GrantProof,
}

impl GrantDirectory {
    pub fn insert(&mut self, grant_id: impl Into<String>, record: GrantRecord) {
        self.grants.insert(grant_id.into(), record);
    }

    pub(crate) fn authorize(
        &self,
        proof: &GrantProof,
        expected_owner: &OpaqueOwnerRef,
        expected_actor: &OpaquePrincipalRef,
    ) -> Result<(), PolicyError> {
        if !valid_key_id(&proof.grant_id) || proof.generation == 0 {
            return Err(PolicyError::InvalidGrant);
        }
        let grant = self
            .grants
            .get(&proof.grant_id)
            .ok_or(PolicyError::AuthorizationDenied)?;
        if grant.revoked
            || grant.generation != proof.generation
            || &grant.owner != expected_owner
            || &grant.authorized_actor != expected_actor
        {
            return Err(PolicyError::AuthorizationDenied);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyError {
    AuthorizationDenied,
    InvalidGrant,
    ForbiddenContent,
}

pub(crate) fn valid_key_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value.contains("..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b':'))
}

pub(crate) fn scan_for_forbidden_content<T: serde::Serialize>(
    value: &T,
) -> Result<(), PolicyError> {
    let bytes = canonical_bytes(value).map_err(|_| PolicyError::ForbiddenContent)?;
    let json: CanonicalJson =
        serde_json::from_slice(&bytes).map_err(|_| PolicyError::ForbiddenContent)?;
    scan_json(&json)
}

fn scan_json(value: &CanonicalJson) -> Result<(), PolicyError> {
    match value {
        CanonicalJson::Array(values) => values.iter().try_for_each(scan_json),
        CanonicalJson::Object(entries) => {
            for (key, child) in entries {
                if forbidden_key(key) {
                    return Err(PolicyError::ForbiddenContent);
                }
                scan_json(child)?;
            }
            Ok(())
        }
        CanonicalJson::String(text) => {
            if forbidden_value(text) {
                Err(PolicyError::ForbiddenContent)
            } else {
                Ok(())
            }
        }
        _ => Ok(()),
    }
}

fn forbidden_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase().replace(['-', '_', ' ', '.'], "");
    FORBIDDEN_KEYS
        .iter()
        .map(|candidate| candidate.replace(['-', '_', '.', ' '], ""))
        .any(|candidate| normalized == candidate || normalized.ends_with(&candidate))
}

fn forbidden_value(value: &str) -> bool {
    let normalized = value.to_ascii_lowercase();
    FORBIDDEN_VALUES
        .iter()
        .any(|candidate| normalized.contains(candidate))
}

#[cfg(test)]
mod policy_tests {
    use super::*;
    use capsule_surface_model::canonical::CanonicalJson;
    use std::collections::BTreeMap;

    #[test]
    fn forbidden_fields_and_paths_are_rejected() {
        let mut object = BTreeMap::new();
        object.insert("home_path".to_owned(), CanonicalJson::Null);
        assert_eq!(
            scan_json(&CanonicalJson::Object(object)),
            Err(PolicyError::ForbiddenContent)
        );

        let mut value = BTreeMap::new();
        value.insert("note".to_owned(), CanonicalJson::String("home://x".into()));
        assert_eq!(
            scan_json(&CanonicalJson::Object(value)),
            Err(PolicyError::ForbiddenContent)
        );
    }

    #[test]
    fn key_ids_reject_injection() {
        assert!(!valid_key_id("../escape"));
        assert!(!valid_key_id("a/b"));
        assert!(!valid_key_id("a\0b"));
        assert!(valid_key_id("patch-1:ok_id"));
    }
}
