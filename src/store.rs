//! Identity store + KV backend abstraction.
//!
//! The [`Backend`] trait is the substitution seam — production uses
//! [`SdkBackend`] (calls the SDK `kv::*` host fns); unit tests use an
//! in-memory `BTreeMap` so the logic can be exercised on the host
//! target without a WASM runtime.
//!
//! ## KV key scheme
//!
//! Keys mirror the legacy kernel store byte-for-byte:
//!
//! * `user/{uuid}`                        → JSON [`AstridUser`]
//! * `link/{platform}/{platform_user_id}` → JSON [`FrontendLink`]
//! * `name/{display_name}`                → UTF-8 UUID string (last-writer-wins index)
//!
//! `platform` and `platform_user_id` are validated to reject `/` and
//! `\0`. Without that gate, a caller passing `platform = "../user"`
//! could read or overwrite a `user/{uuid}` record through the link path.

use astrid_sdk::prelude::kv;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::time::now_rfc3339;
use crate::types::{AstridUser, FrontendLink, StoreError, normalize_platform};

/// KV operations the store needs.
pub trait Backend {
    /// Return the raw bytes for `key`, or `None` if absent.
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, String>;
    /// Store `value` at `key`.
    fn set(&self, key: &str, value: &[u8]) -> Result<(), String>;
    /// Remove `key`. Idempotent.
    fn delete(&self, key: &str) -> Result<(), String>;
    /// Every key whose name starts with `prefix`.
    fn list_keys(&self, prefix: &str) -> Result<Vec<String>, String>;
}

/// Live backend that calls the SDK `kv::*` host fns.
#[derive(Default)]
pub struct SdkBackend;

impl Backend for SdkBackend {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        match kv::get_bytes(key) {
            // SDK's get_bytes returns Ok(vec![]) for both absent and
            // empty values. Every value the store writes is JSON
            // (minimum 2 bytes), so empty == missing.
            Ok(bytes) if bytes.is_empty() => Ok(None),
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) => Err(e.to_string()),
        }
    }
    fn set(&self, key: &str, value: &[u8]) -> Result<(), String> {
        kv::set_bytes(key, value).map_err(|e| e.to_string())
    }
    fn delete(&self, key: &str) -> Result<(), String> {
        kv::delete(key).map_err(|e| e.to_string())
    }
    fn list_keys(&self, prefix: &str) -> Result<Vec<String>, String> {
        kv::list_keys(prefix).map_err(|e| e.to_string())
    }
}

/// Identity store over a [`Backend`].
pub struct Store<B: Backend> {
    backend: B,
}

impl<B: Backend> Store<B> {
    #[must_use]
    pub fn new(backend: B) -> Self {
        Self { backend }
    }

    pub(crate) fn user_key(id: Uuid) -> String {
        format!("user/{id}")
    }
    pub(crate) fn link_key(platform: &str, platform_user_id: &str) -> String {
        format!("link/{platform}/{platform_user_id}")
    }
    pub(crate) fn name_key(name: &str) -> String {
        format!("name/{name}")
    }

    pub(crate) fn validate_non_empty(value: &str, field: &str) -> Result<(), StoreError> {
        if value.trim().is_empty() {
            return Err(StoreError::InvalidInput(format!(
                "{field} must not be empty"
            )));
        }
        Ok(())
    }

    pub(crate) fn validate_key_component(value: &str, field: &str) -> Result<(), StoreError> {
        Self::validate_non_empty(value, field)?;
        if value.contains('/') || value.contains('\0') {
            return Err(StoreError::InvalidInput(format!(
                "{field} must not contain '/' or null bytes"
            )));
        }
        Ok(())
    }

    fn get_json<T: for<'de> Deserialize<'de>>(&self, key: &str) -> Result<Option<T>, StoreError> {
        let bytes = self.backend.get(key).map_err(StoreError::Storage)?;
        match bytes {
            None => Ok(None),
            Some(b) => serde_json::from_slice(&b)
                .map(Some)
                .map_err(|e| StoreError::Storage(format!("decode {key}: {e}"))),
        }
    }

    fn set_json<T: Serialize>(&self, key: &str, value: &T) -> Result<(), StoreError> {
        let bytes = serde_json::to_vec(value)
            .map_err(|e| StoreError::Storage(format!("encode {key}: {e}")))?;
        self.backend.set(key, &bytes).map_err(StoreError::Storage)
    }

    /// Create a new user. Writes the name index for non-empty names
    /// (last-writer-wins, mirroring the kernel store).
    pub fn create_user(&self, display_name: Option<&str>) -> Result<AstridUser, StoreError> {
        if let Some(name) = display_name
            && (name.contains('/') || name.contains('\0'))
        {
            return Err(StoreError::InvalidInput(
                "display_name must not contain '/' or null bytes".into(),
            ));
        }
        let user = AstridUser::new(display_name.map(str::to_string));
        self.set_json(&Self::user_key(user.id), &user)?;
        if let Some(name) = display_name {
            let trimmed = name.trim();
            if !trimmed.is_empty() {
                self.backend
                    .set(&Self::name_key(trimmed), user.id.to_string().as_bytes())
                    .map_err(StoreError::Storage)?;
            }
        }
        Ok(user)
    }

    /// Fetch a user by UUID.
    pub fn get_user(&self, id: Uuid) -> Result<Option<AstridUser>, StoreError> {
        self.get_json(&Self::user_key(id))
    }

    /// Resolve `(platform, platform_user_id)` to the linked user.
    pub fn resolve(
        &self,
        platform: &str,
        platform_user_id: &str,
    ) -> Result<Option<AstridUser>, StoreError> {
        Self::validate_key_component(platform, "platform")?;
        Self::validate_key_component(platform_user_id, "platform_user_id")?;
        let normalized = normalize_platform(platform);
        let key = Self::link_key(&normalized, platform_user_id);
        let link: Option<FrontendLink> = self.get_json(&key)?;
        match link {
            None => Ok(None),
            Some(l) => self.get_user(l.astrid_user_id),
        }
    }

    /// Upsert a platform link. The target user must exist.
    pub fn link(
        &self,
        platform: &str,
        platform_user_id: &str,
        astrid_user_id: Uuid,
        method: &str,
    ) -> Result<FrontendLink, StoreError> {
        Self::validate_key_component(platform, "platform")?;
        Self::validate_key_component(platform_user_id, "platform_user_id")?;
        Self::validate_non_empty(method, "method")?;

        if self.get_user(astrid_user_id)?.is_none() {
            return Err(StoreError::UserNotFound(astrid_user_id));
        }

        let normalized = normalize_platform(platform);
        let link = FrontendLink {
            platform: normalized.clone(),
            platform_user_id: platform_user_id.to_string(),
            astrid_user_id,
            linked_at: now_rfc3339(),
            method: method.to_string(),
        };
        self.set_json(&Self::link_key(&normalized, platform_user_id), &link)?;
        Ok(link)
    }

    /// Remove a link. Returns `true` if a link existed.
    pub fn unlink(&self, platform: &str, platform_user_id: &str) -> Result<bool, StoreError> {
        Self::validate_key_component(platform, "platform")?;
        Self::validate_key_component(platform_user_id, "platform_user_id")?;
        let normalized = normalize_platform(platform);
        let key = Self::link_key(&normalized, platform_user_id);
        let existed = self
            .backend
            .get(&key)
            .map_err(StoreError::Storage)?
            .is_some();
        if existed {
            self.backend.delete(&key).map_err(StoreError::Storage)?;
        }
        Ok(existed)
    }

    /// List every link pointing at `astrid_user_id`.
    pub fn list_links(&self, astrid_user_id: Uuid) -> Result<Vec<FrontendLink>, StoreError> {
        let keys = self
            .backend
            .list_keys("link/")
            .map_err(StoreError::Storage)?;
        let mut out = Vec::new();
        for k in keys {
            if let Some(l) = self.get_json::<FrontendLink>(&k)?
                && l.astrid_user_id == astrid_user_id
            {
                out.push(l);
            }
        }
        Ok(out)
    }

    /// Delete a user record + every link pointing at it. Idempotent.
    pub fn delete_user(&self, id: Uuid) -> Result<bool, StoreError> {
        let Some(user) = self.get_user(id)? else {
            return Ok(false);
        };

        // Cascade: strip every link aimed at this user.
        let link_keys = self
            .backend
            .list_keys("link/")
            .map_err(StoreError::Storage)?;
        for k in link_keys {
            if let Some(l) = self.get_json::<FrontendLink>(&k)?
                && l.astrid_user_id == id
            {
                self.backend.delete(&k).map_err(StoreError::Storage)?;
            }
        }

        // Drop the name index entry only when it still points at this UUID.
        // The name index is best-effort last-writer-wins; if another user
        // overwrote it, this user's delete must not clobber the new entry.
        if let Some(name) = user.display_name.as_deref() {
            let key = Self::name_key(name.trim());
            if let Some(bytes) = self.backend.get(&key).map_err(StoreError::Storage)?
                && String::from_utf8(bytes).ok().as_deref() == Some(id.to_string().as_str())
            {
                self.backend.delete(&key).map_err(StoreError::Storage)?;
            }
        }

        self.backend
            .delete(&Self::user_key(id))
            .map_err(StoreError::Storage)?;
        Ok(true)
    }

    /// List every user record in the store.
    pub fn list_users(&self) -> Result<Vec<AstridUser>, StoreError> {
        let keys = self
            .backend
            .list_keys("user/")
            .map_err(StoreError::Storage)?;
        let mut out = Vec::with_capacity(keys.len());
        for k in keys {
            if let Some(u) = self.get_json::<AstridUser>(&k)? {
                out.push(u);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::Backend;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    #[derive(Default)]
    pub(crate) struct MemBackend {
        pub(crate) inner: RefCell<BTreeMap<String, Vec<u8>>>,
    }

    impl Backend for MemBackend {
        fn get(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
            Ok(self.inner.borrow().get(key).cloned())
        }
        fn set(&self, key: &str, value: &[u8]) -> Result<(), String> {
            self.inner.borrow_mut().insert(key.into(), value.to_vec());
            Ok(())
        }
        fn delete(&self, key: &str) -> Result<(), String> {
            self.inner.borrow_mut().remove(key);
            Ok(())
        }
        fn list_keys(&self, prefix: &str) -> Result<Vec<String>, String> {
            Ok(self
                .inner
                .borrow()
                .keys()
                .filter(|k| k.starts_with(prefix))
                .cloned()
                .collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::MemBackend;
    use super::*;

    fn make_store() -> Store<MemBackend> {
        Store::new(MemBackend::default())
    }

    #[test]
    fn create_and_get_user() {
        let store = make_store();
        let u = store.create_user(Some("Alice")).unwrap();
        assert_eq!(u.display_name.as_deref(), Some("Alice"));
        assert_eq!(store.get_user(u.id).unwrap(), Some(u));
    }

    #[test]
    fn create_user_no_name_writes_no_index() {
        let store = make_store();
        let u = store.create_user(None).unwrap();
        assert!(u.display_name.is_none());
        assert!(store.backend.list_keys("name/").unwrap().is_empty());
    }

    #[test]
    fn create_user_rejects_slash_in_name() {
        assert!(matches!(
            make_store().create_user(Some("admin/root")),
            Err(StoreError::InvalidInput(_))
        ));
    }

    #[test]
    fn create_user_rejects_null_in_name() {
        assert!(matches!(
            make_store().create_user(Some("oops\0me")),
            Err(StoreError::InvalidInput(_))
        ));
    }

    #[test]
    fn get_user_missing_returns_none() {
        assert!(make_store().get_user(Uuid::new_v4()).unwrap().is_none());
    }

    #[test]
    fn link_then_resolve_round_trips() {
        let store = make_store();
        let u = store.create_user(Some("Bob")).unwrap();
        store.link("Discord", "12345", u.id, "admin").unwrap();
        assert_eq!(store.resolve("discord", "12345").unwrap().unwrap().id, u.id);
    }

    #[test]
    fn resolve_normalizes_platform_lookup() {
        let store = make_store();
        let u = store.create_user(None).unwrap();
        store.link("  DISCORD  ", "abc", u.id, "admin").unwrap();
        assert_eq!(store.resolve("Discord", "abc").unwrap().unwrap().id, u.id);
        assert_eq!(store.resolve("discord", "abc").unwrap().unwrap().id, u.id);
    }

    #[test]
    fn resolve_missing_returns_none() {
        assert!(make_store().resolve("discord", "ghost").unwrap().is_none());
    }

    #[test]
    fn link_rejects_missing_user() {
        let err = make_store()
            .link("discord", "123", Uuid::new_v4(), "admin")
            .unwrap_err();
        assert!(matches!(err, StoreError::UserNotFound(_)));
    }

    #[test]
    fn link_upsert_overwrites() {
        let store = make_store();
        let a = store.create_user(Some("Alice")).unwrap();
        let b = store.create_user(Some("Bob")).unwrap();
        store.link("discord", "123", a.id, "admin").unwrap();
        store.link("discord", "123", b.id, "admin").unwrap();
        assert_eq!(store.resolve("discord", "123").unwrap().unwrap().id, b.id);
    }

    #[test]
    fn link_rejects_empty_method() {
        let store = make_store();
        let u = store.create_user(None).unwrap();
        assert!(matches!(
            store.link("discord", "1", u.id, ""),
            Err(StoreError::InvalidInput(_))
        ));
    }

    #[test]
    fn link_rejects_slash_in_platform() {
        let store = make_store();
        let u = store.create_user(None).unwrap();
        assert!(matches!(
            store.link("a/b", "1", u.id, "admin"),
            Err(StoreError::InvalidInput(_))
        ));
    }

    #[test]
    fn link_rejects_slash_in_platform_user_id() {
        let store = make_store();
        let u = store.create_user(None).unwrap();
        assert!(matches!(
            store.link("discord", "1/../../etc", u.id, "admin"),
            Err(StoreError::InvalidInput(_))
        ));
    }

    #[test]
    fn link_rejects_null_in_inputs() {
        let store = make_store();
        let u = store.create_user(None).unwrap();
        assert!(matches!(
            store.link("disc\0rd", "1", u.id, "admin"),
            Err(StoreError::InvalidInput(_))
        ));
        assert!(matches!(
            store.link("discord", "1\0", u.id, "admin"),
            Err(StoreError::InvalidInput(_))
        ));
    }

    #[test]
    fn unlink_removes_link() {
        let store = make_store();
        let u = store.create_user(None).unwrap();
        store.link("telegram", "789", u.id, "admin").unwrap();
        assert!(store.unlink("telegram", "789").unwrap());
        assert!(store.resolve("telegram", "789").unwrap().is_none());
    }

    #[test]
    fn unlink_idempotent_when_missing() {
        assert!(!make_store().unlink("discord", "ghost").unwrap());
    }

    #[test]
    fn list_links_filters_by_user() {
        let store = make_store();
        let a = store.create_user(Some("Alice")).unwrap();
        let b = store.create_user(Some("Bob")).unwrap();
        store.link("discord", "a1", a.id, "admin").unwrap();
        store.link("telegram", "a2", a.id, "admin").unwrap();
        store.link("discord", "b1", b.id, "admin").unwrap();
        let alinks = store.list_links(a.id).unwrap();
        assert_eq!(alinks.len(), 2);
        assert!(alinks.iter().all(|l| l.astrid_user_id == a.id));
        assert_eq!(store.list_links(b.id).unwrap().len(), 1);
    }

    #[test]
    fn list_links_empty_for_unknown_user() {
        assert!(make_store().list_links(Uuid::new_v4()).unwrap().is_empty());
    }

    #[test]
    fn delete_user_cascades_links() {
        let store = make_store();
        let a = store.create_user(Some("Alice")).unwrap();
        store.link("discord", "a1", a.id, "admin").unwrap();
        store.link("telegram", "a2", a.id, "admin").unwrap();
        assert!(store.delete_user(a.id).unwrap());
        assert!(store.get_user(a.id).unwrap().is_none());
        assert!(store.resolve("discord", "a1").unwrap().is_none());
        assert!(store.resolve("telegram", "a2").unwrap().is_none());
    }

    #[test]
    fn delete_user_idempotent() {
        assert!(!make_store().delete_user(Uuid::new_v4()).unwrap());
    }

    #[test]
    fn delete_user_preserves_other_users_links() {
        let store = make_store();
        let a = store.create_user(Some("Alice")).unwrap();
        let b = store.create_user(Some("Bob")).unwrap();
        store.link("discord", "a1", a.id, "admin").unwrap();
        store.link("discord", "b1", b.id, "admin").unwrap();
        assert!(store.delete_user(a.id).unwrap());
        assert_eq!(store.resolve("discord", "b1").unwrap().unwrap().id, b.id);
    }

    #[test]
    fn delete_clears_name_index_only_when_still_pointed_at_uuid() {
        let store = make_store();
        let a = store.create_user(Some("Shared")).unwrap();
        let b = store.create_user(Some("Shared")).unwrap();
        assert!(store.delete_user(a.id).unwrap());
        let bytes = store.backend.get("name/Shared").unwrap().unwrap();
        assert_eq!(String::from_utf8(bytes).unwrap(), b.id.to_string());
    }

    #[test]
    fn list_users_returns_all() {
        let store = make_store();
        let a = store.create_user(Some("a")).unwrap();
        let b = store.create_user(Some("b")).unwrap();
        let c = store.create_user(None).unwrap();
        let mut got = store.list_users().unwrap();
        got.sort_by_key(|u| u.id);
        let mut want = vec![a, b, c];
        want.sort_by_key(|u| u.id);
        assert_eq!(got, want);
    }

    #[test]
    fn list_users_excludes_deleted() {
        let store = make_store();
        let a = store.create_user(Some("a")).unwrap();
        let b = store.create_user(Some("b")).unwrap();
        store.delete_user(a.id).unwrap();
        let users = store.list_users().unwrap();
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].id, b.id);
    }

    #[test]
    fn validate_key_component_rejects_traversal_attempts() {
        assert!(matches!(
            Store::<MemBackend>::validate_key_component("../user", "platform"),
            Err(StoreError::InvalidInput(_))
        ));
    }
}
