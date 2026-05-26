use astrid_sdk::net::{TcpStream, TryRecvError, bind_unix};
use astrid_sdk::prelude::*;

#[derive(Default)]
struct CliProxy;

#[capsule]
impl CliProxy {
    #[astrid::run]
    fn run(&self) -> Result<(), SysError> {
        // 1. Subscribe to TUI-relevant IPC topics only.
        // IMPORTANT: If a new event topic is consumed by the TUI, add it here.
        // Internal pipeline events (LLM requests, tool dispatch, identity builds)
        // must NOT be forwarded to the CLI socket.
        let topics = [
            "agent.v1.response",
            "astrid.v1.onboarding.required",
            "astrid.v1.elicit.*",
            "astrid.v1.approval",
            "astrid.v1.response.*",
            "astrid.v1.capsules_loaded",
            "registry.v1.response.*",
            "registry.v1.active_model_changed",
            "registry.v1.selection.*",
            "session.v1.response.*",
        ];
        // Subscriptions are RAII handles - drop releases the kernel-side
        // resource. Keep them owned by the run loop for the proxy's lifetime.
        let subs: Vec<ipc::Subscription> = topics
            .iter()
            .map(|t| ipc::subscribe(t))
            .collect::<Result<Vec<_>, _>>()?;

        // Signal readiness so the kernel can proceed with loading dependent capsules.
        // Best-effort: failure means the host mutex is poisoned (unrecoverable).
        let _ = runtime::signal_ready();

        // 2. Resolve the socket path from the kernel-injected config.
        // bind_unix is a no-op on the host side (the kernel pre-binds the socket),
        // but the path is used for logging and future diagnostics.
        let path = runtime::socket_path()
            .map_err(|e| SysError::ApiError(format!("Failed to resolve socket path: {e}")))?;

        log::info(format!("CLI Proxy: accepting connections on {path}"));
        let listener = bind_unix()?;

        // 3. Multi-connection accept loop.
        // Supports up to 8 concurrent CLI clients (enforced at host level).
        // IPC events are broadcast to all connected clients. Any authenticated
        // client can send prompts - the daemon is a single agent.
        //
        // TcpStream is the post-#752 unified handle (Unix-domain accepts and
        // outbound TCP share the same resource type). Drop releases the
        // kernel-side stream entry, so we no longer need a manual close.
        let mut streams: Vec<TcpStream> = Vec::new();

        'proxy: loop {
            // Phase A: block until at least one client is connected.
            if streams.is_empty() {
                let stream = match listener.accept() {
                    Ok(s) => s,
                    Err(e) => {
                        log::warn(format!("Accept error: {e:?}, backing off"));
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        continue;
                    }
                };
                log::info("CLI client connected to proxy");
                streams.push(stream);
            }

            // Phase B: poll for one additional connection (non-blocking).
            // Max one per iteration to bound handshake stall to ~5s worst case.
            // The new try_accept takes a timeout - 0 means non-blocking, matching
            // the pre-#752 semantics.
            if let Ok(Some(new_stream)) = listener.try_accept(0) {
                log::info("Additional CLI client connected to proxy");
                streams.push(new_stream);
            }

            // Phase C: read from all streams.
            // NOTE: 50ms timeout per stream = linear scaling (N*50ms per iteration).
            // Acceptable for CLI use (2-3 typical, 8 max = 400ms worst case).
            let mut dead_indices: Vec<usize> = Vec::new();
            for (i, stream) in streams.iter().enumerate() {
                match stream.try_recv() {
                    Ok(bytes) => handle_ingress(&bytes),
                    Err(TryRecvError::Empty) => {}
                    Err(TryRecvError::Closed) => {
                        log::info("CLI client disconnected from proxy");
                        dead_indices.push(i);
                    }
                }
            }

            // Remove dead streams in reverse order to preserve indices.
            // Drop releases the host-side active_streams entry automatically -
            // no explicit close() needed (the pre-#752 manual close was a
            // workaround for the lack of resource Drop in the old ABI).
            for &i in dead_indices.iter().rev() {
                streams.remove(i);
            }

            // Phase D: poll IPC subscriptions and broadcast to all live streams.
            // NOTE: broadcast_dead indices are into streams AFTER Phase C removals.
            let mut broadcast_dead: Vec<usize> = Vec::new();
            for sub in &subs {
                match sub.poll() {
                    Ok(result) => {
                        if !result.messages.is_empty() {
                            broadcast_poll_messages(&streams, &result, &mut broadcast_dead);
                        }
                    }
                    Err(_) => {
                        log::error("IPC subscription error, proxy shutting down");
                        break 'proxy;
                    }
                }
            }

            // Remove streams that failed during broadcast.
            // Multiple subscriptions may flag the same stream as dead in one
            // iteration. sort + dedup before removal prevents double-removal panics.
            broadcast_dead.sort_unstable();
            broadcast_dead.dedup();
            for &i in broadcast_dead.iter().rev() {
                streams.remove(i);
                log::info("CLI client disconnected during broadcast");
            }
        }

        // Reached only when an IPC subscription fails (break 'proxy above).
        Err(SysError::ApiError(
            "IPC subscription failed, proxy terminated".to_string(),
        ))
    }
}

/// Parse an incoming client message and publish it to the IPC bus if the
/// topic passes the ingress allowlist.
fn handle_ingress(bytes: &[u8]) {
    let msg = match serde_json::from_slice::<serde_json::Value>(bytes) {
        Ok(v) => v,
        Err(_) => {
            log::warn("Received malformed IPC payload from socket");
            return;
        }
    };

    let (Some(topic), Some(payload)) = (
        msg.get("topic").and_then(|t| t.as_str()),
        msg.get("payload"),
    ) else {
        log::warn("Dropped ingress message: missing topic or payload");
        return;
    };

    if is_allowed_ingress_topic(topic) {
        if let Err(e) = ipc::publish_json(topic, payload) {
            log::error(format!("Failed to publish IPC: {e:?}"));
        }
    } else {
        log::warn(format!("Dropped ingress message to blocked topic: {topic}"));
    }
}

/// Broadcast each IPC message from a `PollResult` to every connected stream.
/// Tracks failed stream indices in `dead`.
fn broadcast_poll_messages(
    streams: &[TcpStream],
    poll_result: &ipc::PollResult,
    dead: &mut Vec<usize>,
) {
    if poll_result.dropped > 0 {
        log::warn(format!(
            "Event bus dropped {} messages - TUI may be stale",
            poll_result.dropped
        ));
    }

    // Pre-serialize each message once, then write to all streams.
    // Reconstruct the wire format the TUI expects: {topic, payload, source_id}.
    let serialized: Vec<Vec<u8>> = poll_result
        .messages
        .iter()
        .filter_map(|msg| {
            // Parse the payload string back to a JSON value so the TUI
            // receives an embedded object, not an escaped string.
            let payload = serde_json::from_str::<serde_json::Value>(&msg.payload)
                .unwrap_or(serde_json::Value::String(msg.payload.clone()));
            serde_json::to_vec(&serde_json::json!({
                "topic": msg.topic,
                "payload": payload,
                "source_id": msg.source_id,
            }))
            .ok()
        })
        .collect();

    for (i, stream) in streams.iter().enumerate() {
        // Skip streams already marked dead by a previous subscription's broadcast.
        if dead.contains(&i) {
            continue;
        }
        for msg_bytes in &serialized {
            if let Err(e) = stream.send(msg_bytes) {
                log::warn(format!(
                    "Socket send error, client likely disconnected: {e:?}"
                ));
                dead.push(i);
                break; // Skip remaining messages for this dead stream.
            }
        }
    }
}

/// Exact topics the CLI is allowed to publish to the internal IPC bus.
/// Note: `client.v1.disconnect` is NOT here - the authoritative disconnect
/// event is published by the kernel-side stream-close path (TcpStream Drop)
/// to avoid double-counting in the idle monitor.
const ALLOWED_INGRESS_EXACT: &[&str] = &["user.v1.prompt", "cli.v1.command.execute"];

/// Topic prefixes the CLI is allowed to publish (suffix-routed topics).
/// IMPORTANT: Update this list when adding new CLI-originated topic prefixes.
const ALLOWED_INGRESS_PREFIXES: &[&str] = &[
    "astrid.v1.request.",
    "astrid.v1.elicit.response.",
    "astrid.v1.approval.response.",
    "registry.v1.selection.",
    "session.v1.request.",
];

fn is_allowed_ingress_topic(topic: &str) -> bool {
    ALLOWED_INGRESS_EXACT.contains(&topic)
        || ALLOWED_INGRESS_PREFIXES
            .iter()
            .any(|p| topic.starts_with(p))
}
