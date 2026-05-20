#![deny(unsafe_code)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]

//! Within-principal user identity store for Astrid OS.
//!
//! Implements the `astrid:users@1.0.0` interface
//! (`unicity-astrid/wit/interfaces/users.wit`) over IPC RPC. Each
//! operation is a request/response pair on the IPC bus, correlated by
//! the requester-supplied `correlation-id`:
//!
//! * `users.v1.resolve.{request,response}`  — platform-id → user lookup.
//! * `users.v1.link.{request,response}`     — upsert a platform link.
//! * `users.v1.unlink.{request,response}`   — remove a platform link.
//! * `users.v1.create.{request,response}`   — create a new user.
//! * `users.v1.links.{request,response}`    — list links for one user.
//! * `users.v1.get.{request,response}`      — fetch one user by UUID.
//! * `users.v1.delete.{request,response}`   — delete user + every link.
//! * `users.v1.list.{request,response}`     — list every user record.
//!
//! Records live in the capsule's per-principal KV scope, keyed
//! identically to the legacy kernel `astrid-storage::identity` store so
//! a future cutover (unicity-astrid/astrid#747) can read existing data
//! unchanged. Modules:
//!
//! * [`types`]      — domain records (`AstridUser`, `FrontendLink`, `Source`).
//! * [`store`]      — KV-backed identity store + `Backend` substitution seam.
//! * [`requests`]   — inbound IPC payload shapes.
//! * [`responses`]  — outbound JSON projection helpers.
//! * [`time`]       — RFC 3339 helpers.

use astrid_sdk::prelude::*;
use uuid::Uuid;

mod requests;
mod responses;
mod store;
mod time;
mod types;

pub use requests::{
    CreateRequest, DeleteRequest, GetRequest, LinkRequest, LinksRequest, ListRequest,
    ResolveRequest, UnlinkRequest,
};
pub use responses::{link_to_json, user_to_json};
pub use store::{Backend, SdkBackend, Store};
pub use types::{AstridUser, FrontendLink, Source, StoreError, normalize_platform};

/// The users capsule entry point. Stateless — each handler builds a
/// fresh [`Store`] backed by the SDK KV host fns.
#[derive(Default)]
pub struct UsersCapsule;

impl UsersCapsule {
    fn store(&self) -> Store<SdkBackend> {
        Store::new(SdkBackend)
    }

    fn publish_error(topic: &str, correlation_id: &str, err: StoreError) {
        let _ = ipc::publish_json(
            topic,
            &serde_json::json!({
                "correlation-id": correlation_id,
                "error": err.to_string(),
            }),
        );
    }

    fn publish(topic: &str, payload: serde_json::Value) {
        let _ = ipc::publish_json(topic, &payload);
    }

    fn parse_uuid(s: &str, topic: &str, cid: &str) -> Option<Uuid> {
        match Uuid::parse_str(s) {
            Ok(u) => Some(u),
            Err(e) => {
                Self::publish_error(
                    topic,
                    cid,
                    StoreError::InvalidInput(format!("astrid_user_id is not a valid UUID: {e}")),
                );
                None
            }
        }
    }
}

#[capsule]
impl UsersCapsule {
    /// `users.v1.resolve.request` → `users.v1.resolve.response`.
    #[astrid::interceptor("handle_resolve")]
    pub fn handle_resolve(&self, req: ResolveRequest) -> Result<(), SysError> {
        const TOPIC: &str = "users.v1.resolve.response";
        let cid = &req.source.correlation_id;
        match self.store().resolve(&req.platform, &req.platform_user_id) {
            Ok(user) => Self::publish(
                TOPIC,
                serde_json::json!({
                    "correlation-id": cid,
                    "user": user_to_json(user.as_ref()),
                }),
            ),
            Err(e) => Self::publish_error(TOPIC, cid, e),
        }
        Ok(())
    }

    /// `users.v1.link.request` → `users.v1.link.response`.
    #[astrid::interceptor("handle_link")]
    pub fn handle_link(&self, req: LinkRequest) -> Result<(), SysError> {
        const TOPIC: &str = "users.v1.link.response";
        let cid = &req.source.correlation_id;
        let Some(astrid_id) = Self::parse_uuid(&req.astrid_user_id, TOPIC, cid) else {
            return Ok(());
        };
        match self
            .store()
            .link(&req.platform, &req.platform_user_id, astrid_id, &req.method)
        {
            Ok(link) => Self::publish(
                TOPIC,
                serde_json::json!({
                    "correlation-id": cid,
                    "link": link_to_json(&link),
                }),
            ),
            Err(e) => Self::publish_error(TOPIC, cid, e),
        }
        Ok(())
    }

    /// `users.v1.unlink.request` → `users.v1.unlink.response`.
    #[astrid::interceptor("handle_unlink")]
    pub fn handle_unlink(&self, req: UnlinkRequest) -> Result<(), SysError> {
        const TOPIC: &str = "users.v1.unlink.response";
        let cid = &req.source.correlation_id;
        match self.store().unlink(&req.platform, &req.platform_user_id) {
            Ok(removed) => Self::publish(
                TOPIC,
                serde_json::json!({
                    "correlation-id": cid,
                    "removed": removed,
                }),
            ),
            Err(e) => Self::publish_error(TOPIC, cid, e),
        }
        Ok(())
    }

    /// `users.v1.create.request` → `users.v1.create.response`.
    #[astrid::interceptor("handle_create")]
    pub fn handle_create(&self, req: CreateRequest) -> Result<(), SysError> {
        const TOPIC: &str = "users.v1.create.response";
        let cid = &req.source.correlation_id;
        match self.store().create_user(req.display_name.as_deref()) {
            Ok(user) => Self::publish(
                TOPIC,
                serde_json::json!({
                    "correlation-id": cid,
                    "user": user_to_json(Some(&user)),
                }),
            ),
            Err(e) => Self::publish_error(TOPIC, cid, e),
        }
        Ok(())
    }

    /// `users.v1.links.request` → `users.v1.links.response`.
    #[astrid::interceptor("handle_links")]
    pub fn handle_links(&self, req: LinksRequest) -> Result<(), SysError> {
        const TOPIC: &str = "users.v1.links.response";
        let cid = &req.source.correlation_id;
        let Some(astrid_id) = Self::parse_uuid(&req.astrid_user_id, TOPIC, cid) else {
            return Ok(());
        };
        match self.store().list_links(astrid_id) {
            Ok(links) => Self::publish(
                TOPIC,
                serde_json::json!({
                    "correlation-id": cid,
                    "links": links.iter().map(link_to_json).collect::<Vec<_>>(),
                }),
            ),
            Err(e) => Self::publish_error(TOPIC, cid, e),
        }
        Ok(())
    }

    /// `users.v1.get.request` → `users.v1.get.response`.
    #[astrid::interceptor("handle_get")]
    pub fn handle_get(&self, req: GetRequest) -> Result<(), SysError> {
        const TOPIC: &str = "users.v1.get.response";
        let cid = &req.source.correlation_id;
        let Some(astrid_id) = Self::parse_uuid(&req.astrid_user_id, TOPIC, cid) else {
            return Ok(());
        };
        match self.store().get_user(astrid_id) {
            Ok(user) => Self::publish(
                TOPIC,
                serde_json::json!({
                    "correlation-id": cid,
                    "user": user_to_json(user.as_ref()),
                }),
            ),
            Err(e) => Self::publish_error(TOPIC, cid, e),
        }
        Ok(())
    }

    /// `users.v1.delete.request` → `users.v1.delete.response`.
    #[astrid::interceptor("handle_delete")]
    pub fn handle_delete(&self, req: DeleteRequest) -> Result<(), SysError> {
        const TOPIC: &str = "users.v1.delete.response";
        let cid = &req.source.correlation_id;
        let Some(astrid_id) = Self::parse_uuid(&req.astrid_user_id, TOPIC, cid) else {
            return Ok(());
        };
        match self.store().delete_user(astrid_id) {
            Ok(deleted) => Self::publish(
                TOPIC,
                serde_json::json!({
                    "correlation-id": cid,
                    "deleted": deleted,
                }),
            ),
            Err(e) => Self::publish_error(TOPIC, cid, e),
        }
        Ok(())
    }

    /// `users.v1.list.request` → `users.v1.list.response`.
    #[astrid::interceptor("handle_list")]
    pub fn handle_list(&self, req: ListRequest) -> Result<(), SysError> {
        const TOPIC: &str = "users.v1.list.response";
        let cid = &req.source.correlation_id;
        match self.store().list_users() {
            Ok(users) => Self::publish(
                TOPIC,
                serde_json::json!({
                    "correlation-id": cid,
                    "users": users
                        .iter()
                        .filter_map(|u| user_to_json(Some(u)))
                        .collect::<Vec<_>>(),
                }),
            ),
            Err(e) => Self::publish_error(TOPIC, cid, e),
        }
        Ok(())
    }
}
