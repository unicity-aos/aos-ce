#![deny(unsafe_code)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]
#![allow(missing_docs)]

//! System management tools capsule for Unicity AOS.
//!
//! Gives the LLM typed tools to inspect and manage its own runtime.
//! All operations go through the kernel's VFS and capability system —
//! the capsule cannot bypass sandbox boundaries.
//!
//! # Tools
//!
//! - `list_capsules` — enumerate the principal's last loaded capsule snapshot
//! - `inspect_capsule` — read metadata referencing a shared capsule artifact
//! - `list_interfaces` — list available WIT interface contracts
//! - `read_interface` — read a WIT interface definition
//! - `system_status` — runtime health and interface coverage summary

use astrid_sdk::prelude::*;
use astrid_sdk::schemars;
use serde::{Deserialize, Serialize};

mod inventory;

/// Standard WIT interface directory — per-principal, accessible via `home://wit/`.
const WIT_DIR: &str = "home://wit";

#[derive(Default)]
pub struct SystemTools;

// ---------------------------------------------------------------------------
// Tool argument types
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct EmptyArgs {}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct InspectCapsuleArgs {
    /// Capsule name (e.g. `aos-session`).
    pub name: String,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct ReadInterfaceArgs {
    /// Interface filename (e.g. `session.wit`).
    pub name: String,
}

// ---------------------------------------------------------------------------
// Response types (serialized to JSON for the LLM)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct CapsuleSummary {
    name: String,
    version: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    exports: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    imports: Vec<String>,
}

#[derive(Debug, Serialize)]
struct SystemStatusResponse {
    view: &'static str,
    principal: String,
    observed_at: String,
    capsule_count: usize,
    exports: Vec<String>,
    imports_satisfied: Vec<String>,
    imports_unsatisfied: Vec<String>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract `namespace/interface` strings from the nested exports/imports map
/// in meta.json: `{ "astrid": { "session": "1.0.0" } }` → `["astrid/session 1.0.0"]`
fn flatten_interface_map(map: &serde_json::Value) -> Vec<String> {
    let mut result = Vec::new();
    if let Some(obj) = map.as_object() {
        for (ns, ifaces) in obj {
            if let Some(ifaces_obj) = ifaces.as_object() {
                for (name, version) in ifaces_obj {
                    let ver = version.as_str().unwrap_or("?");
                    result.push(format!("{ns}/{name} {ver}"));
                }
            }
        }
    }
    result
}

/// List entry names under a VFS path.
fn list_entries(path: &str) -> Result<Vec<String>, SysError> {
    let entries = astrid_sdk::fs::read_dir(path)?;
    let mut names: Vec<String> = entries.map(|e| e.file_name().to_string()).collect();
    names.sort();
    Ok(names)
}

// ---------------------------------------------------------------------------
// Tool implementations
// ---------------------------------------------------------------------------

#[capsule]
impl SystemTools {
    /// Receive the runtime-stamped, principal-scoped loaded metadata view.
    #[astrid::interceptor("receive_inventory")]
    pub fn receive_inventory(&self, payload: serde_json::Value) -> Result<(), SysError> {
        inventory::receive(payload)
    }

    /// List capsules in the last runtime-loaded snapshot for this principal. Use `inspect_capsule`
    /// for its metadata and shared WASM identity, exports, and imports.
    /// Returns a JSON array of capsule summaries.
    #[astrid::tool("list_capsules")]
    pub fn list_capsules(&self, _args: EmptyArgs) -> Result<String, SysError> {
        let snapshot = inventory::load()?;
        let mut summaries = Vec::new();

        for name in snapshot.capsules.keys() {
            let meta = snapshot.metadata(name)?;
            let version = meta
                .get("version")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();

            let exports = meta
                .get("exports")
                .map(flatten_interface_map)
                .unwrap_or_default();

            let imports = meta
                .get("imports")
                .map(flatten_interface_map)
                .unwrap_or_default();

            summaries.push(CapsuleSummary {
                name: name.clone(),
                version,
                exports,
                imports,
            });
        }

        serde_json::to_string_pretty(&summaries)
            .map_err(|e| SysError::ApiError(format!("serialize: {e}")))
    }

    /// Read loaded capsule metadata, including its shared WASM identity.
    /// Returns the last observed principal snapshot entry, not a private manifest copy.
    #[astrid::tool("inspect_capsule")]
    pub fn inspect_capsule(&self, args: InspectCapsuleArgs) -> Result<String, SysError> {
        let name = args.name.trim();
        if name.is_empty() {
            return Err(SysError::ApiError("Capsule name cannot be empty".into()));
        }

        // Reject path traversal
        if name.contains('/') || name.contains('\\') || name.contains("..") {
            return Err(SysError::ApiError(
                "Invalid capsule name — path traversal rejected".into(),
            ));
        }

        let snapshot = inventory::load()?;
        let metadata = snapshot.metadata(name)?;
        serde_json::to_string_pretty(&serde_json::json!({
            "view": "last_observed_loaded_capsules",
            "principal": snapshot.principal,
            "observed_at": snapshot.observed_at,
            "name": name,
            "metadata": metadata,
        }))
        .map_err(|e| SysError::ApiError(format!("serialize: {e}")))
    }

    /// List all WIT interface definitions available in the system.
    /// These define the typed contracts between capsules.
    #[astrid::tool("list_interfaces")]
    pub fn list_interfaces(&self, _args: EmptyArgs) -> Result<String, SysError> {
        match list_entries(WIT_DIR) {
            Ok(files) => {
                if files.is_empty() {
                    Ok("No WIT interfaces installed. Run `aos init` to set up the standard interfaces.".into())
                } else {
                    Ok(files.join("\n"))
                }
            }
            Err(_) => Ok(
                "WIT directory not found. Run `aos init` to set up the standard interfaces.".into(),
            ),
        }
    }

    /// Read a WIT interface definition file. Returns the full typed contract
    /// so you can understand the message schemas between capsules.
    #[astrid::tool("read_interface")]
    pub fn read_interface(&self, args: ReadInterfaceArgs) -> Result<String, SysError> {
        let name = args.name.trim();
        if name.is_empty() {
            return Err(SysError::ApiError("Interface name cannot be empty".into()));
        }

        // Reject path traversal
        if name.contains('/') || name.contains('\\') || name.contains("..") {
            return Err(SysError::ApiError(
                "Invalid interface name — path traversal rejected".into(),
            ));
        }

        // Add .wit extension if not present
        let filename = if name.ends_with(".wit") {
            name.to_string()
        } else {
            format!("{name}.wit")
        };

        let path = format!("{WIT_DIR}/{filename}");
        astrid_sdk::fs::read_to_string(&path).map_err(|_| {
            SysError::ApiError(format!(
                "Interface '{filename}' not found. Use list_interfaces to see available interfaces."
            ))
        })
    }

    /// Show the last loaded principal snapshot: capsule count, interface coverage, satisfied and
    /// unsatisfied imports. Helps you understand the health of the system.
    #[astrid::tool("system_status")]
    pub fn system_status(&self, _args: EmptyArgs) -> Result<String, SysError> {
        let snapshot = inventory::load()?;

        // Collect all exports and imports across all capsules
        let mut all_exports: Vec<String> = Vec::new();
        let mut all_imports: Vec<(String, String)> = Vec::new(); // (interface, capsule_name)

        for name in snapshot.capsules.keys() {
            let meta = snapshot.metadata(name)?;
            if let Some(exports) = meta.get("exports") {
                for iface in flatten_interface_map(exports) {
                    // Strip version for matching: "astrid/session 1.0.0" → "astrid/session"
                    let key = iface.split_whitespace().next().unwrap_or(&iface);
                    if !all_exports.contains(&key.to_string()) {
                        all_exports.push(key.to_string());
                    }
                }
            }

            if let Some(imports) = meta.get("imports") {
                for iface in flatten_interface_map(imports) {
                    let key = iface.split_whitespace().next().unwrap_or(&iface);
                    all_imports.push((key.to_string(), name.clone()));
                }
            }
        }

        let mut satisfied = Vec::new();
        let mut unsatisfied = Vec::new();

        for (iface, capsule) in &all_imports {
            if all_exports.contains(iface) {
                satisfied.push(format!("{iface} (needed by {capsule})"));
            } else {
                unsatisfied.push(format!("{iface} (needed by {capsule})"));
            }
        }

        let status = SystemStatusResponse {
            view: "last_observed_loaded_capsules",
            principal: snapshot.principal.clone(),
            observed_at: snapshot.observed_at.clone(),
            capsule_count: snapshot.capsules.len(),
            exports: all_exports,
            imports_satisfied: satisfied,
            imports_unsatisfied: unsatisfied,
        };

        serde_json::to_string_pretty(&status)
            .map_err(|e| SysError::ApiError(format!("serialize: {e}")))
    }
}
