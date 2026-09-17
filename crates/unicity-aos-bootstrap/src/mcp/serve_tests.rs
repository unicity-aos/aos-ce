use super::*;
use serde_json::json;

fn initialize(capabilities: Value) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-11-25",
            "capabilities": capabilities,
            "clientInfo": { "name": "test", "version": "1" }
        }
    })
    .to_string()
}

fn prepare(frame: &[u8], mode: InteractionMode, supported: &mut bool) -> Option<Vec<u8>> {
    let mut mrtr = mrtr::NativeMrtr::new();
    match prepare_client_message(frame, mode, supported, &mut mrtr) {
        UpstreamPrepare::Rewrite(frame) => Some(frame),
        UpstreamPrepare::Unchanged => None,
        UpstreamPrepare::Reply(_) => panic!("unexpected tools/call host reply"),
        UpstreamPrepare::Reject(error) => panic!("unexpected tools/call reject: {error}"),
    }
}

fn tools_call(id: Value, name: &str, arguments: Value) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": name,
            "arguments": arguments,
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": "2025-11-25",
                "io.modelcontextprotocol/clientCapabilities": { "elicitation": {} }
            }
        }
    })
    .to_string()
}

#[test]
fn auto_advertises_form_only_when_client_cannot_present_it() {
    let mut supported = false;
    let forwarded = prepare(
        initialize(json!({ "roots": {} })).as_bytes(),
        InteractionMode::Auto,
        &mut supported,
    )
    .expect("initialize is transformed");
    let forwarded: Value = serde_json::from_slice(&forwarded).expect("json");
    assert!(!supported);
    assert!(
        forwarded
            .pointer("/params/capabilities/elicitation/form")
            .is_some()
    );

    for mode in [InteractionMode::Auto, InteractionMode::Native] {
        let mut supported = false;
        let forwarded = prepare(
            initialize(json!({ "elicitation": { "form": {} } })).as_bytes(),
            mode,
            &mut supported,
        );
        assert!(supported);
        assert!(
            forwarded.is_none(),
            "{mode:?} must preserve already-capable initialize bytes"
        );
    }
}

#[test]
fn client_and_deny_modes_never_invent_capabilities() {
    for mode in [InteractionMode::Client, InteractionMode::Deny] {
        let mut supported = false;
        let forwarded = prepare(initialize(json!({})).as_bytes(), mode, &mut supported);
        assert!(
            forwarded.is_none(),
            "{mode:?} must preserve unchanged initialize bytes"
        );
    }
}

#[test]
fn malformed_initialize_capabilities_are_not_rewritten() {
    let malformed = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "capabilities": "not-an-object" }
    })
    .to_string();
    let mut supported = false;
    let forwarded = prepare(malformed.as_bytes(), InteractionMode::Auto, &mut supported);
    assert!(
        forwarded.is_none(),
        "malformed capabilities must preserve raw bytes"
    );
    assert!(!supported);
}

#[test]
fn auto_intercepts_only_when_the_client_lacks_form_support() {
    let request = json!({
        "jsonrpc": "2.0",
        "id": 7,
        "method": "elicitation/create",
        "params": { "mode": "form" }
    });
    assert_eq!(
        elicitation_handling(&request, InteractionMode::Auto, false),
        ElicitationHandling::Present
    );
    assert_eq!(
        elicitation_handling(&request, InteractionMode::Auto, true),
        ElicitationHandling::Forward
    );
    assert_eq!(
        elicitation_handling(&request, InteractionMode::Native, true),
        ElicitationHandling::Present
    );
}

#[test]
fn url_elicitation_is_never_intercepted_as_a_local_form() {
    let request = json!({
        "method": "elicitation/create",
        "params": { "mode": "url" }
    });
    assert_eq!(
        elicitation_handling(&request, InteractionMode::Native, false),
        ElicitationHandling::Forward
    );
}

#[test]
fn deny_mode_cancels_form_and_url_elicitation() {
    for request in [
        json!({ "method": "elicitation/create", "params": { "mode": "form" } }),
        json!({ "method": "elicitation/create", "params": { "mode": "url" } }),
    ] {
        assert_eq!(
            elicitation_handling(&request, InteractionMode::Deny, true),
            ElicitationHandling::Cancel
        );
    }
}

#[test]
fn initialize_response_is_product_branded() {
    let mut response = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": {
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "serverInfo": { "name": "astrid", "version": "0.10.4" }
        }
    });
    assert!(rewrite_server_identity(&mut response));
    assert_eq!(response["result"]["serverInfo"]["name"], "unicity-aos");
    assert_eq!(response["result"]["serverInfo"]["title"], "Unicity AOS");
}

#[test]
fn native_tracks_tools_call_and_advertises_per_request_form() {
    let mut supported = false;
    let mut session = mrtr::NativeMrtr::new();
    let request = json!({
        "jsonrpc": "2.0",
        "id": 7,
        "method": "tools/call",
        "params": {
            "name": "fs.read",
            "arguments": { "path": "/tmp/report" },
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": "2025-11-25",
                "io.modelcontextprotocol/clientCapabilities": { "elicitation": {} }
            }
        }
    })
    .to_string();
    let forwarded = match prepare_client_message(
        request.as_bytes(),
        InteractionMode::Native,
        &mut supported,
        &mut session,
    ) {
        UpstreamPrepare::Rewrite(frame) => frame,
        other => panic!("per-request form is advertised, got {other:?}"),
    };
    let forwarded: Value = serde_json::from_slice(&forwarded).expect("json");
    assert_eq!(
        forwarded["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"],
        "2025-11-25"
    );
    assert!(
        forwarded["params"]["_meta"]["io.modelcontextprotocol/clientCapabilities"]["elicitation"]
            ["form"]
            .is_object()
    );
    assert!(!session.is_empty());
}

#[test]
fn native_does_not_invent_request_meta_or_protocol_version() {
    let mut supported = false;
    let mut session = mrtr::NativeMrtr::new();
    let request = json!({
        "jsonrpc": "2.0",
        "id": 7,
        "method": "tools/call",
        "params": {
            "name": "fs.read",
            "arguments": { "path": "/tmp/report" }
        }
    })
    .to_string();
    let forwarded = prepare_client_message(
        request.as_bytes(),
        InteractionMode::Native,
        &mut supported,
        &mut session,
    );
    assert_eq!(
        forwarded,
        UpstreamPrepare::Unchanged,
        "missing _meta must keep original bytes"
    );
    assert!(!session.is_empty());
}

#[test]
fn client_mode_does_not_track_or_rewrite_tools_call() {
    let mut supported = false;
    let mut session = mrtr::NativeMrtr::new();
    let request = json!({
        "jsonrpc": "2.0",
        "id": 7,
        "method": "tools/call",
        "params": {
            "name": "fs.read",
            "_meta": { "io.modelcontextprotocol/clientCapabilities": { "elicitation": {} } }
        }
    })
    .to_string();
    let forwarded = prepare_client_message(
        request.as_bytes(),
        InteractionMode::Client,
        &mut supported,
        &mut session,
    );
    assert_eq!(forwarded, UpstreamPrepare::Unchanged);
    assert!(session.is_empty());
}

#[test]
fn deny_tracks_tools_call_without_inventing_form_meta() {
    let mut supported = false;
    let mut session = mrtr::NativeMrtr::new();
    let request = json!({
        "jsonrpc": "2.0",
        "id": 7,
        "method": "tools/call",
        "params": {
            "name": "fs.read",
            "_meta": { "io.modelcontextprotocol/clientCapabilities": { "elicitation": {} } }
        }
    })
    .to_string();
    let forwarded = prepare_client_message(
        request.as_bytes(),
        InteractionMode::Deny,
        &mut supported,
        &mut session,
    );
    assert_eq!(forwarded, UpstreamPrepare::Unchanged);
    assert!(!session.is_empty());
}

#[test]
fn native_duplicate_tools_call_id_is_rejected_without_replacing_tracking() {
    let mut supported = false;
    let mut session = mrtr::NativeMrtr::new();
    let first = tools_call(json!(7), "fs.read", json!({ "path": "/tmp/report" }));
    match prepare_client_message(
        first.as_bytes(),
        InteractionMode::Native,
        &mut supported,
        &mut session,
    ) {
        UpstreamPrepare::Rewrite(_) => {}
        other => panic!("first tools/call should be tracked, got {other:?}"),
    }

    let second = tools_call(json!(7), "fs.write", json!({ "path": "/etc/passwd" }));
    let prepared = prepare_client_message(
        second.as_bytes(),
        InteractionMode::Native,
        &mut supported,
        &mut session,
    );
    assert_eq!(
        prepared,
        UpstreamPrepare::Reject(mrtr::MrtrError::DuplicateId)
    );
    assert!(!session.is_empty());

    let resume = session
        .decline(&json!({
            "jsonrpc": "2.0",
            "id": 7,
            "result": {
                "resultType": "input_required",
                "requestState": "opaque-token",
                "inputRequests": {
                    "astrid-consent": {
                        "method": "elicitation/create",
                        "params": {
                            "mode": "form",
                            "message": "Allow this capsule to continue?",
                            "requestedSchema": {
                                "type": "object",
                                "properties": { "grant": { "type": "boolean" } },
                                "required": ["grant"]
                            }
                        }
                    }
                }
            }
        }))
        .expect("original call remains tracked");
    assert_eq!(resume["params"]["name"], "fs.read");
    assert_eq!(resume["params"]["arguments"]["path"], "/tmp/report");
}

#[test]
fn intercepting_malformed_tools_call_is_rejected_without_tracking() {
    let requests = [
        json!({
            "jsonrpc": "2.0",
            "id": true,
            "method": "tools/call",
            "params": { "name": "fs.read" }
        }),
        json!({
            "jsonrpc": "2.0",
            "id": null,
            "method": "tools/call",
            "params": { "name": "fs.read" }
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 1.5,
            "method": "tools/call",
            "params": { "name": "fs.read" }
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": {}
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": "not-an-object"
        }),
        json!({
            "jsonrpc": "2.0",
            "method": "tools/call",
            "params": { "name": "fs.read" }
        }),
    ];
    for request in requests {
        for mode in [InteractionMode::Native, InteractionMode::Deny] {
            let mut supported = false;
            let mut session = mrtr::NativeMrtr::new();
            let prepared = prepare_client_message(
                request.to_string().as_bytes(),
                mode,
                &mut supported,
                &mut session,
            );
            assert!(
                matches!(
                    prepared,
                    UpstreamPrepare::Reject(mrtr::MrtrError::Malformed(_))
                ),
                "{mode:?} must reject malformed tools/call {request}"
            );
            assert!(
                session.is_empty(),
                "{mode:?} must not track malformed tools/call"
            );
        }
    }
}

#[test]
fn client_mode_bytepasses_duplicate_and_malformed_tools_call() {
    let mut supported = false;
    let mut session = mrtr::NativeMrtr::new();
    let first = tools_call(json!(7), "fs.read", json!({ "path": "/tmp/report" }));
    assert_eq!(
        prepare_client_message(
            first.as_bytes(),
            InteractionMode::Client,
            &mut supported,
            &mut session,
        ),
        UpstreamPrepare::Unchanged
    );
    let duplicate = tools_call(json!(7), "fs.write", json!({ "path": "/etc/passwd" }));
    assert_eq!(
        prepare_client_message(
            duplicate.as_bytes(),
            InteractionMode::Client,
            &mut supported,
            &mut session,
        ),
        UpstreamPrepare::Unchanged
    );
    let malformed = json!({
        "jsonrpc": "2.0",
        "id": true,
        "method": "tools/call",
        "params": { "name": "fs.read" }
    })
    .to_string();
    assert_eq!(
        prepare_client_message(
            malformed.as_bytes(),
            InteractionMode::Client,
            &mut supported,
            &mut session,
        ),
        UpstreamPrepare::Unchanged
    );
    assert!(session.is_empty());
}

#[test]
fn native_cancel_notification_forwards_and_drops_tracking() {
    let mut supported = false;
    let mut session = mrtr::NativeMrtr::new();
    let request = tools_call(json!(7), "fs.read", json!({ "path": "/tmp/report" }));
    match prepare_client_message(
        request.as_bytes(),
        InteractionMode::Native,
        &mut supported,
        &mut session,
    ) {
        UpstreamPrepare::Rewrite(_) => {}
        other => panic!("tools/call should be tracked, got {other:?}"),
    }
    let cancel = json!({
        "jsonrpc": "2.0",
        "method": "notifications/cancelled",
        "params": { "requestId": 7 }
    })
    .to_string();
    let prepared = prepare_client_message(
        cancel.as_bytes(),
        InteractionMode::Native,
        &mut supported,
        &mut session,
    );
    assert_eq!(prepared, UpstreamPrepare::Unchanged);
    assert!(session.is_empty());
}

fn serve_args(interaction: InteractionMode, socket: Option<&str>) -> ServeArgs {
    ServeArgs {
        interaction,
        workspace: None,
        request_timeout: None,
        interaction_socket: socket.map(PathBuf::from),
        interaction_timeout: interaction::DEFAULT_INTERACTION_TIMEOUT_SECONDS,
        max_in_flight_calls: mrtr::DEFAULT_MAX_IN_FLIGHT_CALLS,
        max_input_rounds: mrtr::DEFAULT_MAX_INPUT_ROUNDS,
    }
}

#[test]
fn interaction_socket_requires_explicit_native_mode() {
    for mode in [
        InteractionMode::Auto,
        InteractionMode::Client,
        InteractionMode::Deny,
    ] {
        let error = native_surface(&serve_args(mode, Some("/tmp/aos-tray.sock")))
            .expect_err("socket without native");
        assert!(
            error.contains("--interaction native"),
            "{mode:?} must reject --interaction-socket: {error}"
        );
    }
}

#[test]
fn native_without_socket_keeps_the_platform_presenter() {
    assert_eq!(
        native_surface(&serve_args(InteractionMode::Native, None)).expect("native default"),
        NativeSurface::Platform
    );
    assert_eq!(
        native_surface(&serve_args(InteractionMode::Auto, None)).expect("auto default"),
        NativeSurface::Platform
    );
}

#[cfg(unix)]
#[test]
fn native_socket_selects_the_tray_surface() {
    assert_eq!(
        native_surface(&serve_args(
            InteractionMode::Native,
            Some("/tmp/aos-tray.sock")
        ))
        .expect("native socket"),
        NativeSurface::Socket(PathBuf::from("/tmp/aos-tray.sock"))
    );
}

#[test]
fn native_excess_in_flight_call_replies_without_dropping_existing() {
    let mut supported = false;
    let mut session = mrtr::NativeMrtr::with_limits(1, mrtr::DEFAULT_MAX_INPUT_ROUNDS);
    let first = tools_call(json!(7), "fs.read", json!({ "path": "/tmp/report" }));
    match prepare_client_message(
        first.as_bytes(),
        InteractionMode::Native,
        &mut supported,
        &mut session,
    ) {
        UpstreamPrepare::Rewrite(_) => {}
        other => panic!("first tools/call should be tracked, got {other:?}"),
    }

    let second = tools_call(json!(8), "fs.write", json!({ "path": "/tmp/other" }));
    let prepared = prepare_client_message(
        second.as_bytes(),
        InteractionMode::Native,
        &mut supported,
        &mut session,
    );
    match prepared {
        UpstreamPrepare::Reply(frame) => {
            let value: Value = serde_json::from_slice(&frame).expect("json");
            assert_eq!(value["id"], 8);
            assert_eq!(
                value["error"]["message"],
                "too many in-flight native tools/call invocations"
            );
        }
        other => panic!("excess tools/call must reply to the host, got {other:?}"),
    }

    let resume = session
        .decline(&json!({
            "jsonrpc": "2.0",
            "id": 7,
            "result": {
                "resultType": "input_required",
                "requestState": "opaque-token",
                "inputRequests": {
                    "astrid-consent": {
                        "method": "elicitation/create",
                        "params": {
                            "mode": "form",
                            "message": "Allow this capsule to continue?",
                            "requestedSchema": {
                                "type": "object",
                                "properties": { "grant": { "type": "boolean" } },
                                "required": ["grant"]
                            }
                        }
                    }
                }
            }
        }))
        .expect("original call remains tracked");
    assert_eq!(resume["params"]["name"], "fs.read");
    assert_eq!(resume["params"]["arguments"]["path"], "/tmp/report");
}

struct AcceptingPresenter;

impl interaction::Presenter for AcceptingPresenter {
    fn present(
        &mut self,
        _request: &interaction::InteractionRequest,
    ) -> Result<Option<usize>, interaction::InteractionError> {
        Ok(Some(0))
    }
}

fn input_required_round(state: &str) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": 7,
        "result": {
            "resultType": "input_required",
            "requestState": state,
            "inputRequests": {
                "astrid-consent": {
                    "method": "elicitation/create",
                    "params": {
                        "mode": "form",
                        "message": "Allow this capsule to continue?",
                        "requestedSchema": {
                            "type": "object",
                            "properties": { "grant": { "type": "boolean" } },
                            "required": ["grant"]
                        }
                    }
                }
            }
        }
    })
    .to_string()
}

#[test]
fn native_excess_rounds_settle_with_host_error_and_free_the_slot() {
    let mut supported = false;
    let mut session = mrtr::NativeMrtr::with_limits(1, 2);
    let request = tools_call(json!(7), "fs.read", json!({ "path": "/tmp/report" }));
    match prepare_client_message(
        request.as_bytes(),
        InteractionMode::Native,
        &mut supported,
        &mut session,
    ) {
        UpstreamPrepare::Rewrite(_) => {}
        other => panic!("tools/call should be tracked, got {other:?}"),
    }

    let mut presenter = AcceptingPresenter;
    for state in ["round-a", "round-b"] {
        match intercept_downstream(
            input_required_round(state).as_bytes(),
            InteractionMode::Native,
            supported,
            &mut session,
            &mut presenter,
        ) {
            DownstreamIntercept::Resume(resume) => {
                assert_eq!(resume["params"]["name"], "fs.read");
                assert_eq!(resume["params"]["requestState"], state);
            }
            DownstreamIntercept::Swallow => panic!("{state} must resume, not swallow"),
            DownstreamIntercept::HostError(_) => panic!("{state} must resume, not host-error"),
            DownstreamIntercept::Exhausted { .. } => {
                panic!("{state} must resume, not settle as exhausted")
            }
            DownstreamIntercept::None => panic!("{state} must resume, not fall through"),
        }
    }

    match intercept_downstream(
        input_required_round("round-c").as_bytes(),
        InteractionMode::Native,
        supported,
        &mut session,
        &mut presenter,
    ) {
        DownstreamIntercept::Exhausted {
            response,
            runtime_cancel,
        } => {
            assert_eq!(response["id"], 7);
            assert_eq!(response["error"]["code"], -32603);
            assert_eq!(
                response["error"]["message"],
                "tracked tools/call exceeded the native input round limit"
            );
            assert_eq!(runtime_cancel["method"], "notifications/cancelled");
            assert_eq!(runtime_cancel["params"]["requestId"], 7);
        }
        DownstreamIntercept::Swallow => panic!("excess rounds must not be swallowed"),
        DownstreamIntercept::HostError(_) => {
            panic!("excess rounds must cancel the runtime, not only error the host")
        }
        DownstreamIntercept::Resume(_) => panic!("excess rounds must not resume"),
        DownstreamIntercept::None => panic!("excess rounds must not fall through"),
    }
    assert!(session.is_empty(), "exhausted call must release its slot");

    let next = tools_call(json!(8), "fs.write", json!({ "path": "/tmp/other" }));
    match prepare_client_message(
        next.as_bytes(),
        InteractionMode::Native,
        &mut supported,
        &mut session,
    ) {
        UpstreamPrepare::Rewrite(_) => {}
        other => panic!("reclaimed slot must accept the next tools/call, got {other:?}"),
    }
    assert!(!session.is_empty());
}

#[test]
fn initialize_response_with_correct_identity_is_not_rewritten() {
    let mut response = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": {
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "serverInfo": {
                "name": "unicity-aos",
                "title": "Unicity AOS",
                "version": env!("CARGO_PKG_VERSION")
            }
        }
    });
    assert!(!rewrite_server_identity(&mut response));
}
