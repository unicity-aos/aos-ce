//! Identity store + KV backend abstraction.
//!
//! The [`Backend`] trait is the substitution seam — production uses
//! [`SdkBackend`] (calls the SDK `kv::*` host fns); unit tests use an
//! in-memory `BTreeMap` so the logic can be exercised on the host
//! target without a WASM runtime.
//!
//! ## KV key scheme
//!
//! `_` (single underscore) is the sentinel for `platform_instance =
//! None` — chosen because the validation layer rejects `_` from being
//! a real instance value, so it cannot collide with a legitimate
//! Slack workspace / IRC network name.
//!
//! | Key | Value |
//! |---|---|
//! | `user/{uuid}` | JSON [`AstridUser`] |
//! | `link/{platform}/{instance_or_underscore}/{platform_user_id}` | JSON [`FrontendLink`] |
//! | `name/{display_name}` | UTF-8 UUID string (best-effort lookup index) |
//! | `context/{platform}/{instance_or_underscore}/{context_id}/{platform_user_id}` | JSON [`ContextIdentity`] |
//!
//! `platform`, `platform_user_id`, `platform_instance`, and
//! `context_id` are all validated to reject `/` and `\0` so the key
//! path cannot be injected.

use astrid_sdk::prelude::kv;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::time::now_rfc3339;
use crate::types::{
    AstridUser, ContextIdentity, FrontendLink, ResolvedDisplayName, StoreError, normalize_platform,
};

const INSTANCE_NONE_SENTINEL: &str = "_";

/// Default pagination limit when caller passes `None`.
const DEFAULT_LIST_LIMIT: usize = 100;
const MAX_LIST_LIMIT: usize = 1000;

/// KV operations the store needs.
pub trait Backend {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, String>;
    fn set(&self, key: &str, value: &[u8]) -> Result<(), String>;
    fn delete(&self, key: &str) -> Result<(), String>;
    fn list_keys(&self, prefix: &str) -> Result<Vec<String>, String>;
}

/// Live backend that calls the SDK `kv::*` host fns.
#[derive(Default)]
pub struct SdkBackend;

impl Backend for SdkBackend {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        match kv::get_bytes(key) {
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

/// Page of results.
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
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

    // ── key construction ────────────────────────────────────────────

    fn user_key(id: Uuid) -> String {
        format!("user/{id}")
    }

    fn instance_segment(instance: Option<&str>) -> String {
        instance
            .map(str::to_string)
            .unwrap_or_else(|| INSTANCE_NONE_SENTINEL.to_string())
    }

    fn link_key(platform: &str, instance: Option<&str>, platform_user_id: &str) -> String {
        format!(
            "link/{platform}/{}/{platform_user_id}",
            Self::instance_segment(instance)
        )
    }

    fn name_key(name: &str) -> String {
        format!("name/{name}")
    }

    fn context_key(
        platform: &str,
        instance: Option<&str>,
        context_id: &str,
        platform_user_id: &str,
    ) -> String {
        format!(
            "context/{platform}/{}/{context_id}/{platform_user_id}",
            Self::instance_segment(instance)
        )
    }

    fn context_prefix_in_context(
        platform: &str,
        instance: Option<&str>,
        context_id: &str,
    ) -> String {
        format!(
            "context/{platform}/{}/{context_id}/",
            Self::instance_segment(instance)
        )
    }

    /// Prefix over which to scan to find every context overlay for one
    /// link, regardless of `context_id`. Callers filter by trailing
    /// `/{platform_user_id}` to identify which overlays belong to the
    /// link. Used on unlink/delete-user cascades and list-for-user.
    fn context_prefix_for_link_bucket(platform: &str, instance: Option<&str>) -> String {
        format!("context/{platform}/{}/", Self::instance_segment(instance))
    }

    // ── validation ──────────────────────────────────────────────────

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

    /// Platform-instance values cannot equal the sentinel `"_"` or
    /// contain key-path-breaking characters.
    pub(crate) fn validate_instance(value: Option<&str>) -> Result<(), StoreError> {
        match value {
            None => Ok(()),
            Some(s) => {
                Self::validate_key_component(s, "platform_instance")?;
                if s == INSTANCE_NONE_SENTINEL {
                    return Err(StoreError::InvalidInput(format!(
                        "platform_instance must not be {INSTANCE_NONE_SENTINEL:?} (reserved sentinel)"
                    )));
                }
                Ok(())
            }
        }
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

    // ── user operations ────────────────────────────────────────────

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

    pub fn get_user(&self, id: Uuid) -> Result<Option<AstridUser>, StoreError> {
        self.get_json(&Self::user_key(id))
    }

    pub fn set_display_name(
        &self,
        id: Uuid,
        display_name: Option<&str>,
    ) -> Result<AstridUser, StoreError> {
        if let Some(name) = display_name
            && (name.contains('/') || name.contains('\0'))
        {
            return Err(StoreError::InvalidInput(
                "display_name must not contain '/' or null bytes".into(),
            ));
        }
        let mut user = self.get_user(id)?.ok_or(StoreError::UserNotFound(id))?;

        // Clear old name-index entry if it pointed at us.
        if let Some(old_name) = user.display_name.as_deref() {
            let key = Self::name_key(old_name.trim());
            if let Some(bytes) = self.backend.get(&key).map_err(StoreError::Storage)?
                && String::from_utf8(bytes).ok().as_deref() == Some(id.to_string().as_str())
            {
                self.backend.delete(&key).map_err(StoreError::Storage)?;
            }
        }

        user.display_name = display_name.map(str::to_string);
        self.set_json(&Self::user_key(id), &user)?;

        if let Some(name) = display_name {
            let trimmed = name.trim();
            if !trimmed.is_empty() {
                self.backend
                    .set(&Self::name_key(trimmed), id.to_string().as_bytes())
                    .map_err(StoreError::Storage)?;
            }
        }
        Ok(user)
    }

    pub fn set_public_key(
        &self,
        id: Uuid,
        public_key: Option<[u8; 32]>,
    ) -> Result<AstridUser, StoreError> {
        let mut user = self.get_user(id)?.ok_or(StoreError::UserNotFound(id))?;
        user.public_key = public_key;
        self.set_json(&Self::user_key(id), &user)?;
        Ok(user)
    }

    // ── link operations ────────────────────────────────────────────

    pub fn resolve(
        &self,
        platform: &str,
        instance: Option<&str>,
        platform_user_id: &str,
        context_id: Option<&str>,
    ) -> Result<(Option<AstridUser>, Option<ResolvedDisplayName>), StoreError> {
        Self::validate_key_component(platform, "platform")?;
        Self::validate_instance(instance)?;
        Self::validate_key_component(platform_user_id, "platform_user_id")?;
        if let Some(c) = context_id {
            Self::validate_key_component(c, "context_id")?;
        }
        let normalized = normalize_platform(platform);
        let key = Self::link_key(&normalized, instance, platform_user_id);
        let link: Option<FrontendLink> = self.get_json(&key)?;
        let Some(link) = link else {
            return Ok((None, None));
        };

        let user = self.get_user(link.astrid_user_id)?;

        // Layer display names: context > link > canonical.
        let mut resolved_name: Option<ResolvedDisplayName> = None;
        if let Some(ctx) = context_id {
            let ckey = Self::context_key(&normalized, instance, ctx, platform_user_id);
            if let Some(overlay) = self.get_json::<ContextIdentity>(&ckey)? {
                resolved_name = Some(ResolvedDisplayName {
                    name: overlay.display_name,
                    source: "context",
                });
            }
        }
        if resolved_name.is_none()
            && let Some(name) = &link.display_name
        {
            resolved_name = Some(ResolvedDisplayName {
                name: name.clone(),
                source: "link",
            });
        }
        if resolved_name.is_none()
            && let Some(u) = &user
            && let Some(name) = &u.display_name
        {
            resolved_name = Some(ResolvedDisplayName {
                name: name.clone(),
                source: "canonical",
            });
        }
        Ok((user, resolved_name))
    }

    pub fn link(
        &self,
        platform: &str,
        instance: Option<&str>,
        platform_user_id: &str,
        astrid_user_id: Uuid,
        method: &str,
        display_name: Option<&str>,
    ) -> Result<FrontendLink, StoreError> {
        Self::validate_key_component(platform, "platform")?;
        Self::validate_instance(instance)?;
        Self::validate_key_component(platform_user_id, "platform_user_id")?;
        Self::validate_non_empty(method, "method")?;

        if self.get_user(astrid_user_id)?.is_none() {
            return Err(StoreError::UserNotFound(astrid_user_id));
        }

        let normalized = normalize_platform(platform);
        let link = FrontendLink {
            platform: normalized.clone(),
            platform_instance: instance.map(str::to_string),
            platform_user_id: platform_user_id.to_string(),
            astrid_user_id,
            linked_at: now_rfc3339(),
            method: method.to_string(),
            display_name: display_name.map(str::to_string),
        };
        self.set_json(
            &Self::link_key(&normalized, instance, platform_user_id),
            &link,
        )?;
        Ok(link)
    }

    pub fn unlink(
        &self,
        platform: &str,
        instance: Option<&str>,
        platform_user_id: &str,
    ) -> Result<bool, StoreError> {
        Self::validate_key_component(platform, "platform")?;
        Self::validate_instance(instance)?;
        Self::validate_key_component(platform_user_id, "platform_user_id")?;
        let normalized = normalize_platform(platform);
        let key = Self::link_key(&normalized, instance, platform_user_id);
        let existed = self
            .backend
            .get(&key)
            .map_err(StoreError::Storage)?
            .is_some();
        if existed {
            // Cascade: scan and delete every context overlay for this link.
            // The context prefix `context/{platform}/{instance}/` enumerates
            // every context-id; we filter by trailing platform_user_id.
            let prefix = Self::context_prefix_for_link_bucket(&normalized, instance);
            let suffix = format!("/{platform_user_id}");
            let ctx_keys = self
                .backend
                .list_keys(&prefix)
                .map_err(StoreError::Storage)?;
            for ckey in ctx_keys {
                // Key shape: context/{platform}/{instance}/{context_id}/{platform_user_id}
                if ckey.ends_with(&suffix) {
                    self.backend.delete(&ckey).map_err(StoreError::Storage)?;
                }
            }
            self.backend.delete(&key).map_err(StoreError::Storage)?;
        }
        Ok(existed)
    }

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

    pub fn delete_user(&self, id: Uuid) -> Result<bool, StoreError> {
        let Some(user) = self.get_user(id)? else {
            return Ok(false);
        };

        // Cascade #1: every link aimed at this user.
        let link_keys = self
            .backend
            .list_keys("link/")
            .map_err(StoreError::Storage)?;
        let mut affected_links: Vec<(String, Option<String>, String)> = Vec::new();
        for k in link_keys {
            if let Some(l) = self.get_json::<FrontendLink>(&k)?
                && l.astrid_user_id == id
            {
                affected_links.push((
                    l.platform.clone(),
                    l.platform_instance.clone(),
                    l.platform_user_id.clone(),
                ));
                self.backend.delete(&k).map_err(StoreError::Storage)?;
            }
        }

        // Cascade #2: every context overlay tied to those links.
        for (platform, instance, platform_user_id) in &affected_links {
            let prefix = Self::context_prefix_for_link_bucket(platform, instance.as_deref());
            let suffix = format!("/{platform_user_id}");
            let ctx_keys = self
                .backend
                .list_keys(&prefix)
                .map_err(StoreError::Storage)?;
            for ckey in ctx_keys {
                if ckey.ends_with(&suffix) {
                    self.backend.delete(&ckey).map_err(StoreError::Storage)?;
                }
            }
        }

        // Cascade #3: name-index entry, only if it still points at this UUID.
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

    pub fn list_users_paginated(
        &self,
        cursor: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Page<AstridUser>, StoreError> {
        let mut keys = self
            .backend
            .list_keys("user/")
            .map_err(StoreError::Storage)?;
        keys.sort();
        let start_idx = match cursor {
            None => 0,
            Some(c) => keys
                .iter()
                .position(|k| k.as_str() > c)
                .unwrap_or(keys.len()),
        };
        let take = limit.unwrap_or(DEFAULT_LIST_LIMIT).min(MAX_LIST_LIMIT);
        let end_idx = (start_idx + take).min(keys.len());
        let mut items = Vec::with_capacity(end_idx.saturating_sub(start_idx));
        for k in &keys[start_idx..end_idx] {
            if let Some(u) = self.get_json::<AstridUser>(k)? {
                items.push(u);
            }
        }
        let next_cursor = if end_idx < keys.len() {
            Some(keys[end_idx - 1].clone())
        } else {
            None
        };
        Ok(Page { items, next_cursor })
    }

    // ── context overlay operations ─────────────────────────────────

    fn require_link_exists(
        &self,
        platform: &str,
        instance: Option<&str>,
        platform_user_id: &str,
    ) -> Result<(), StoreError> {
        let key = Self::link_key(platform, instance, platform_user_id);
        if self
            .backend
            .get(&key)
            .map_err(StoreError::Storage)?
            .is_none()
        {
            return Err(StoreError::LinkNotFound);
        }
        Ok(())
    }

    pub fn set_context(
        &self,
        platform: &str,
        instance: Option<&str>,
        platform_user_id: &str,
        context_id: &str,
        display_name: &str,
    ) -> Result<ContextIdentity, StoreError> {
        Self::validate_key_component(platform, "platform")?;
        Self::validate_instance(instance)?;
        Self::validate_key_component(platform_user_id, "platform_user_id")?;
        Self::validate_key_component(context_id, "context_id")?;
        Self::validate_non_empty(display_name, "display_name")?;

        let normalized = normalize_platform(platform);
        self.require_link_exists(&normalized, instance, platform_user_id)?;

        let overlay = ContextIdentity {
            platform: normalized.clone(),
            platform_instance: instance.map(str::to_string),
            platform_user_id: platform_user_id.to_string(),
            context_id: context_id.to_string(),
            display_name: display_name.to_string(),
            updated_at: now_rfc3339(),
        };
        self.set_json(
            &Self::context_key(&normalized, instance, context_id, platform_user_id),
            &overlay,
        )?;
        Ok(overlay)
    }

    pub fn clear_context(
        &self,
        platform: &str,
        instance: Option<&str>,
        platform_user_id: &str,
        context_id: &str,
    ) -> Result<bool, StoreError> {
        Self::validate_key_component(platform, "platform")?;
        Self::validate_instance(instance)?;
        Self::validate_key_component(platform_user_id, "platform_user_id")?;
        Self::validate_key_component(context_id, "context_id")?;
        let normalized = normalize_platform(platform);
        let key = Self::context_key(&normalized, instance, context_id, platform_user_id);
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

    pub fn get_context(
        &self,
        platform: &str,
        instance: Option<&str>,
        platform_user_id: &str,
        context_id: &str,
    ) -> Result<(Option<ContextIdentity>, Option<Uuid>), StoreError> {
        Self::validate_key_component(platform, "platform")?;
        Self::validate_instance(instance)?;
        Self::validate_key_component(platform_user_id, "platform_user_id")?;
        Self::validate_key_component(context_id, "context_id")?;
        let normalized = normalize_platform(platform);
        let overlay: Option<ContextIdentity> = self.get_json(&Self::context_key(
            &normalized,
            instance,
            context_id,
            platform_user_id,
        ))?;
        let link: Option<FrontendLink> =
            self.get_json(&Self::link_key(&normalized, instance, platform_user_id))?;
        Ok((overlay, link.map(|l| l.astrid_user_id)))
    }

    pub fn list_context_for_user(
        &self,
        astrid_user_id: Uuid,
        cursor: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Page<ContextIdentity>, StoreError> {
        // Gather every link this user owns; for each, scan context
        // overlays. O(links × contexts/link). Pagination is over the
        // concatenated stable order.
        let links = self.list_links(astrid_user_id)?;
        let mut overlays: Vec<(String, ContextIdentity)> = Vec::new();
        for link in &links {
            let prefix = Self::context_prefix_for_link_bucket(
                &link.platform,
                link.platform_instance.as_deref(),
            );
            let suffix = format!("/{}", link.platform_user_id);
            let keys = self
                .backend
                .list_keys(&prefix)
                .map_err(StoreError::Storage)?;
            for k in keys {
                if k.ends_with(&suffix)
                    && let Some(c) = self.get_json::<ContextIdentity>(&k)?
                {
                    overlays.push((k, c));
                }
            }
        }
        overlays.sort_by(|a, b| a.0.cmp(&b.0));
        let start_idx = match cursor {
            None => 0,
            Some(c) => overlays
                .iter()
                .position(|(k, _)| k.as_str() > c)
                .unwrap_or(overlays.len()),
        };
        let take = limit.unwrap_or(DEFAULT_LIST_LIMIT).min(MAX_LIST_LIMIT);
        let end_idx = (start_idx + take).min(overlays.len());
        let next_cursor = if end_idx < overlays.len() {
            Some(overlays[end_idx - 1].0.clone())
        } else {
            None
        };
        let items: Vec<ContextIdentity> = overlays[start_idx..end_idx]
            .iter()
            .map(|(_, c)| c.clone())
            .collect();
        Ok(Page { items, next_cursor })
    }

    pub fn list_context_in_context(
        &self,
        platform: &str,
        instance: Option<&str>,
        context_id: &str,
        cursor: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Page<(ContextIdentity, Option<Uuid>)>, StoreError> {
        Self::validate_key_component(platform, "platform")?;
        Self::validate_instance(instance)?;
        Self::validate_key_component(context_id, "context_id")?;
        let normalized = normalize_platform(platform);
        let prefix = Self::context_prefix_in_context(&normalized, instance, context_id);
        let mut keys = self
            .backend
            .list_keys(&prefix)
            .map_err(StoreError::Storage)?;
        keys.sort();
        let start_idx = match cursor {
            None => 0,
            Some(c) => keys
                .iter()
                .position(|k| k.as_str() > c)
                .unwrap_or(keys.len()),
        };
        let take = limit.unwrap_or(DEFAULT_LIST_LIMIT).min(MAX_LIST_LIMIT);
        let end_idx = (start_idx + take).min(keys.len());
        let mut items = Vec::with_capacity(end_idx.saturating_sub(start_idx));
        for k in &keys[start_idx..end_idx] {
            if let Some(c) = self.get_json::<ContextIdentity>(k)? {
                let link: Option<FrontendLink> = self.get_json(&Self::link_key(
                    &c.platform,
                    c.platform_instance.as_deref(),
                    &c.platform_user_id,
                ))?;
                items.push((c, link.map(|l| l.astrid_user_id)));
            }
        }
        let next_cursor = if end_idx < keys.len() {
            Some(keys[end_idx - 1].clone())
        } else {
            None
        };
        Ok(Page { items, next_cursor })
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
