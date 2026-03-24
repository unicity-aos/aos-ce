#![deny(unsafe_code)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]

//! LLM Provider Registry capsule.
//!
//! Discovers available LLM providers via IPC hook fan-out and manages
//! model selection. Provider capsules respond to `llm.v1.request.describe`
//! with their capabilities and routing topics, following the same pattern
//! as tool discovery (`tool.v1.request.describe`).
//!
//! # IPC Protocol
//!
//! **Queries** (publish to these topics, registry responds on `registry.v1.response.*`):
//! - `registry.v1.get_providers` — returns list of available LLM providers
//! - `registry.v1.get_active_model` — returns the currently active model
//! - `registry.v1.set_active_model` — payload: `{"model_id": "..."}`, sets active model
//!
//! **Events** (published by registry):
//! - `registry.v1.active_model_changed` — payload: `ProviderEntry`, emitted on model change

use std::sync::atomic::{AtomicU64, Ordering};

use astrid_sdk::prelude::*;

static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(0);
use serde::{Deserialize, Serialize};

/// The kernel's system session UUID, used to validate IPC messages from the kernel.
const KERNEL_UUID: &str = "00000000-0000-0000-0000-000000000000";

/// A resolved LLM provider with its IPC routing topics.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProviderEntry {
    /// Model ID (e.g. "gpt-5.4", "claude-sonnet-4-20250514").
    id: String,
    /// Human-readable description.
    description: String,
    /// IPC topic to publish LLM requests to.
    request_topic: String,
    /// IPC topic the provider streams responses on.
    stream_topic: String,
    /// Model capabilities (e.g. "text", "vision", "tools").
    capabilities: Vec<String>,
    /// Provider's context window size in tokens, if declared.
    #[serde(skip_serializing_if = "Option::is_none")]
    context_window: Option<u64>,
    /// Provider's max output tokens, if declared.
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u64>,
}

/// The persisted registry state.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct RegistryState {
    providers: Vec<ProviderEntry>,
    active_model_id: Option<String>,
}

const STATE_KEY: &str = "registry_state";

fn load_state() -> RegistryState {
    kv::get_json::<RegistryState>(STATE_KEY).unwrap_or_default()
}

fn save_state(state: &RegistryState) {
    let _ = kv::set_json(STATE_KEY, state);
}

/// Discover LLM providers via IPC hook fan-out.
///
/// Uses `hooks::trigger` with `llm.v1.request.describe` — the kernel fans
/// out to all capsules with matching interceptors and returns a JSON array
/// of responses. Each provider capsule returns `{"providers": [...]}`.
fn discover_providers() -> Vec<ProviderEntry> {
    let request = serde_json::json!({
        "hook": "llm.v1.request.describe",
        "payload": {},
    });
    let request_bytes = match serde_json::to_vec(&request) {
        Ok(b) => b,
        Err(e) => {
            let _ = log::log(
                "warn",
                format!("Failed to serialize provider discovery request: {e}"),
            );
            return Vec::new();
        }
    };
    let response_bytes = match hooks::trigger(&request_bytes) {
        Ok(b) => b,
        Err(e) => {
            let _ = log::log(
                "warn",
                format!("Provider discovery hook trigger failed: {e}"),
            );
            return Vec::new();
        }
    };
    let responses: Vec<serde_json::Value> = match serde_json::from_slice(&response_bytes) {
        Ok(r) => r,
        Err(e) => {
            let _ = log::log(
                "warn",
                format!("Failed to parse provider discovery response: {e}"),
            );
            return Vec::new();
        }
    };

    responses
        .iter()
        .filter_map(|resp| resp.get("providers").and_then(|p| p.as_array()))
        .flatten()
        .filter_map(|p| serde_json::from_value::<ProviderEntry>(p.clone()).ok())
        .collect()
}

/// Check whether a poll envelope contains at least one message from the kernel.
///
/// Used to validate `astrid.v1.capsules_loaded` signals.
fn is_from_kernel(poll_bytes: &[u8]) -> bool {
    let envelope: serde_json::Value = match serde_json::from_slice(poll_bytes) {
        Ok(v) => v,
        Err(_) => return false,
    };
    envelope
        .get("messages")
        .and_then(|m| m.as_array())
        .is_some_and(|msgs| {
            msgs.iter().any(|msg| {
                msg.get("source_id")
                    .and_then(|s| s.as_str())
                    .is_some_and(|s| s == KERNEL_UUID)
            })
        })
}

/// Publish the active model changed event so the react loop (and frontends) can respond.
fn publish_model_changed(provider: &ProviderEntry) {
    let _ = ipc::publish_json("registry.v1.active_model_changed", provider);
}

/// Handle a `registry.v1.get_providers` request.
fn handle_get_providers() {
    let providers = discover_providers();
    let mut state = load_state();

    // Only overwrite providers if discovery returned results.
    // An empty result (timeout, capsule not loaded) must not clobber
    // a previously known-good list, as that would break active_model_id
    // references and cause the TUI to show no models.
    if !providers.is_empty() {
        state.providers = providers;
        save_state(&state);
    } else if state.providers.is_empty() {
        let _ = log::log(
            "warn",
            "Provider discovery returned empty and no cached providers exist",
        );
    }

    let _ = ipc::publish_json("registry.v1.response.get_providers", &state.providers);
}

/// Handle a `registry.v1.get_active_model` request.
fn handle_get_active_model() {
    let state = load_state();
    let active = state
        .active_model_id
        .as_ref()
        .and_then(|id| state.providers.iter().find(|p| &p.id == id));

    let _ = ipc::publish_json("registry.v1.response.get_active_model", &active);
}

/// Handle a `registry.v1.set_active_model` request.
///
/// The payload is the serialized `IpcPayload` from the IPC message.
/// For `IpcPayload::Custom { data }`, the JSON shape is
/// `{"type": "custom", "data": {"model_id": "..."}}`.
fn handle_set_active_model(payload: &serde_json::Value) {
    // Extract model_id from inside the Custom payload's `data` field,
    // falling back to a top-level lookup for forward compatibility.
    let model_id = match payload
        .get("data")
        .and_then(|d| d.get("model_id"))
        .and_then(|v| v.as_str())
        .or_else(|| payload.get("model_id").and_then(|v| v.as_str()))
    {
        Some(id) => id.to_string(),
        None => {
            let _ = ipc::publish_json(
                "registry.v1.response.set_active_model",
                &serde_json::json!({"error": "missing model_id"}),
            );
            return;
        }
    };

    let mut state = load_state();

    // Refresh providers if stale
    if state.providers.is_empty() {
        state.providers = discover_providers();
    }

    if let Some(provider) = state.providers.iter().find(|p| p.id == model_id).cloned() {
        state.active_model_id = Some(model_id);
        save_state(&state);
        publish_model_changed(&provider);
        let _ = ipc::publish_json(
            "registry.v1.response.set_active_model",
            &serde_json::json!({"status": "ok", "active_model": provider}),
        );
    } else {
        let _ = ipc::publish_json(
            "registry.v1.response.set_active_model",
            &serde_json::json!({"error": format!("unknown model: {model_id}")}),
        );
    }
}

/// Clear `active_model_id` if it no longer resolves to a known provider.
///
/// After a reload the provider list may change entirely (e.g. a different
/// capsule version with different model IDs). A stale reference would cause
/// `handle_get_active_model` to return `None` without the frontend knowing
/// the selected model was removed.
fn clear_stale_active_model(state: &mut RegistryState) {
    if let Some(ref id) = state.active_model_id
        && !state.providers.iter().any(|p| &p.id == id)
    {
        let _ = log::log(
            "info",
            format!("Active model '{id}' no longer available after reload, clearing"),
        );
        state.active_model_id = None;
        save_state(state);
    }
}

/// Auto-select the sole provider if exactly one is available.
fn auto_select_if_single(state: &mut RegistryState) {
    if state.providers.len() == 1 && state.active_model_id.is_none() {
        let provider = state.providers[0].clone();
        state.active_model_id = Some(provider.id.clone());
        save_state(state);
        publish_model_changed(&provider);
        let _ = log::log(
            "info",
            format!("Auto-selected sole LLM provider: {}", provider.id),
        );
    }
}

#[derive(Default)]
struct Registry;

#[capsule]
impl Registry {
    #[astrid::run]
    fn run(&self) -> Result<(), SysError> {
        let _ = log::info("Registry capsule starting");

        let sub = ipc::subscribe("registry.v1.*").map_err(|e| SysError::ApiError(e.to_string()))?;

        // Subscribe to CLI command execution so we can handle `/models`.
        let cmd_sub = ipc::subscribe("cli.v1.command.execute")
            .map_err(|e| SysError::ApiError(e.to_string()))?;

        // Subscribe to model selection callbacks from the TUI picker.
        let selection_sub = ipc::subscribe("registry.v1.selection.callback")
            .map_err(|e| SysError::ApiError(e.to_string()))?;

        // Signal readiness so the kernel can proceed with loading dependent capsules.
        // Best-effort: failure means the host mutex is poisoned (unrecoverable).
        let _ = runtime::signal_ready();

        // Single subscription for kernel.capsules_loaded - used for both initial
        // readiness wait AND reload re-discovery in the event loop. Avoids the
        // race window of unsubscribe + resubscribe where a message could be missed.
        let capsules_loaded_sub = ipc::subscribe("astrid.v1.capsules_loaded")
            .map_err(|e| SysError::ApiError(e.to_string()))?;

        // Wait for the kernel to signal that all capsules have been loaded.
        let mut capsules_ready = false;
        if let Ok(bytes) = ipc::recv_bytes(&capsules_loaded_sub, 5000)
            && !bytes.is_empty()
            && is_from_kernel(&bytes)
        {
            capsules_ready = true;
        }

        if !capsules_ready {
            let _ = log::log(
                "warn",
                "Timed out waiting for astrid.v1.capsules_loaded - proceeding with discovery anyway",
            );
        }

        // Now that all capsules are loaded, discover providers via IPC hook.
        let providers = discover_providers();
        let mut state = load_state();
        if !providers.is_empty() {
            state.providers = providers;
            save_state(&state);
        } else if state.providers.is_empty() {
            let _ = log::log(
                "warn",
                "Initial provider discovery returned empty and no cached providers exist",
            );
        }
        clear_stale_active_model(&mut state);
        auto_select_if_single(&mut state);

        // Event loop - blocks on the primary subscription, then drains auxiliary subscriptions.
        loop {
            // Block until a registry message arrives (up to 5s), then drain others.
            match ipc::recv_bytes(&sub, 5000) {
                Ok(bytes) => {
                    if !bytes.is_empty() {
                        handle_poll_envelope(&bytes);
                    }
                }
                Err(_) => break,
            }

            // Drain CLI command execution messages (non-blocking).
            if let Ok(bytes) = ipc::poll_bytes(&cmd_sub)
                && !bytes.is_empty()
            {
                handle_command_envelope(&bytes);
            }

            // Drain model selection callbacks from the TUI picker.
            if let Ok(bytes) = ipc::poll_bytes(&selection_sub)
                && !bytes.is_empty()
            {
                handle_selection_envelope(&bytes);
            }

            // Check for capsule reload events - re-discover providers when
            // the kernel signals that capsules were reloaded (e.g. after /refresh).
            if let Ok(bytes) = ipc::poll_bytes(&capsules_loaded_sub)
                && !bytes.is_empty()
                && is_from_kernel(&bytes)
            {
                let _ = log::info("Capsules reloaded - re-discovering providers");
                let providers = discover_providers();
                let mut state = load_state();
                if !providers.is_empty() {
                    state.providers = providers;
                    save_state(&state);
                    clear_stale_active_model(&mut state);
                    auto_select_if_single(&mut state);
                }
            }
        }

        let _ = ipc::unsubscribe(&sub);
        let _ = ipc::unsubscribe(&cmd_sub);
        let _ = ipc::unsubscribe(&selection_sub);
        let _ = ipc::unsubscribe(&capsules_loaded_sub);

        Ok(())
    }
}

/// Parse the poll envelope and dispatch individual messages.
fn handle_poll_envelope(poll_bytes: &[u8]) {
    let envelope: serde_json::Value = match serde_json::from_slice(poll_bytes) {
        Ok(v) => v,
        Err(_) => return,
    };

    if let Some(dropped) = envelope.get("dropped").and_then(|d| d.as_u64())
        && dropped > 0
    {
        let _ = log::log(
            "warn",
            format!("Event bus dropped {dropped} messages in registry poll"),
        );
    }

    let messages = match envelope.get("messages").and_then(|m| m.as_array()) {
        Some(arr) => arr,
        None => return,
    };

    for msg in messages {
        let topic = match msg.get("topic").and_then(|t| t.as_str()) {
            Some(t) => t,
            None => continue,
        };

        // Skip our own response messages to avoid unnecessary processing.
        if topic.starts_with("registry.v1.response.") || topic == "registry.v1.active_model_changed"
        {
            continue;
        }

        match topic {
            "registry.v1.get_providers" => handle_get_providers(),
            "registry.v1.get_active_model" => handle_get_active_model(),
            "registry.v1.set_active_model" => {
                if let Some(payload) = msg.get("payload") {
                    handle_set_active_model(payload);
                }
            }
            _ => {}
        }
    }
}

/// Parse `cli.v1.command.execute` envelopes and handle `/models`.
fn handle_command_envelope(poll_bytes: &[u8]) {
    let envelope: serde_json::Value = match serde_json::from_slice(poll_bytes) {
        Ok(v) => v,
        Err(_) => return,
    };

    let messages = match envelope.get("messages").and_then(|m| m.as_array()) {
        Some(arr) => arr,
        None => return,
    };

    for msg in messages {
        let payload = match msg.get("payload") {
            Some(p) => p,
            None => continue,
        };

        // IpcPayload::UserInput has `"type": "user_input"` and `"text": "..."`
        let text = payload.get("text").and_then(|t| t.as_str()).unwrap_or("");

        let parts: Vec<&str> = text.split_whitespace().collect();
        let cmd = parts.first().copied().unwrap_or("");

        if cmd == "/models" {
            if parts.len() >= 2 {
                // Direct model switch: `/models <model_id>`
                handle_set_active_model_by_id(parts[1]);
            } else {
                // Show selection picker
                emit_model_selection();
            }
        }
    }
}

/// Parse selection callback envelopes and apply the user's model choice.
fn handle_selection_envelope(poll_bytes: &[u8]) {
    let envelope: serde_json::Value = match serde_json::from_slice(poll_bytes) {
        Ok(v) => v,
        Err(_) => return,
    };

    let messages = match envelope.get("messages").and_then(|m| m.as_array()) {
        Some(arr) => arr,
        None => return,
    };

    for msg in messages {
        let payload = match msg.get("payload") {
            Some(p) => p,
            None => continue,
        };

        // The TUI sends IpcPayload::Custom { data: {"request_id": ..., "selected_id": ...} }
        let selected_id = payload
            .get("data")
            .and_then(|d| d.get("selected_id"))
            .and_then(|v| v.as_str())
            .or_else(|| payload.get("selected_id").and_then(|v| v.as_str()));

        if let Some(model_id) = selected_id {
            handle_set_active_model_by_id(model_id);
        }
    }
}

/// Set the active model by ID (extracted helper for reuse).
fn handle_set_active_model_by_id(model_id: &str) {
    let mut state = load_state();

    if state.providers.is_empty() {
        state.providers = discover_providers();
    }

    if let Some(provider) = state.providers.iter().find(|p| p.id == model_id).cloned() {
        state.active_model_id = Some(model_id.to_string());
        save_state(&state);
        publish_model_changed(&provider);
        let _ = ipc::publish_json(
            "registry.v1.response.set_active_model",
            &serde_json::json!({"status": "ok", "active_model": provider}),
        );
    } else {
        let _ = ipc::publish_json(
            "registry.v1.response.set_active_model",
            &serde_json::json!({"error": format!("unknown model: {model_id}")}),
        );
    }
}

/// Discover providers and emit a `SelectionRequired` IPC payload for the TUI.
fn emit_model_selection() {
    let providers = discover_providers();
    let mut state = load_state();

    if !providers.is_empty() {
        state.providers = providers;
        save_state(&state);
    }

    if state.providers.is_empty() {
        let _ = log::warn("No LLM providers found for /models selection");
        return;
    }

    let options: Vec<serde_json::Value> = state
        .providers
        .iter()
        .map(|p| {
            serde_json::json!({
                "id": p.id,
                "label": p.id,
                "description": p.description,
            })
        })
        .collect();

    let request_id = format!(
        "models-{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed)
    );

    // Emit SelectionRequired payload — the host will deserialize this as
    // IpcPayload::SelectionRequired because it matches the serde shape.
    let selection = serde_json::json!({
        "type": "selection_required",
        "request_id": request_id,
        "title": "Select LLM Model",
        "options": options,
        "callback_topic": "registry.v1.selection.callback",
    });

    let _ = ipc::publish_json("registry.v1.response.models", &selection);
}
