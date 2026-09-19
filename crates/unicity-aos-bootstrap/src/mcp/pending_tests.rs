use super::pending::*;
use super::{interaction, mrtr};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

struct Human(Arc<Mutex<mpsc::Receiver<Option<usize>>>>);
impl interaction::Presenter for Human {
    fn present(
        &mut self,
        _: &interaction::InteractionRequest,
    ) -> Result<Option<usize>, interaction::InteractionError> {
        self.0
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(3))
            .map_err(|_| {
                interaction::InteractionError::Unavailable("test human disconnected".into())
            })
    }
}
fn fixture() -> (PendingApprovals, mpsc::Sender<Option<usize>>) {
    configured(8, Duration::from_secs(10))
}

#[test]
fn adapter_advertises_own_contract_without_losing_unrelated_metadata() {
    let (mut bridge, _) = fixture();
    let mut request = call(1);
    request["params"]["_meta"] = json!({"trace":"keep", "io.modelcontextprotocol/clientCapabilities":{"roots":{"listChanged":true}}});
    let sent = forwarded(bridge.upstream(request));
    assert_eq!(sent["params"]["_meta"]["trace"], "keep");
    assert_eq!(
        sent["params"]["_meta"]["io.modelcontextprotocol/clientCapabilities"]["roots"],
        json!({"listChanged":true})
    );
    let mut malformed = call(2);
    malformed["params"]["_meta"] = json!("not an object");
    assert!(matches!(bridge.upstream(malformed), Upstream::Reply(_)));
}
fn configured(limit: usize, timeout: Duration) -> (PendingApprovals, mpsc::Sender<Option<usize>>) {
    let (tx, rx) = mpsc::channel();
    let rx = Arc::new(Mutex::new(rx));
    (
        PendingApprovals::new(
            Box::new(move || Box::new(Human(rx.clone()))),
            limit,
            mrtr::DEFAULT_MAX_INPUT_ROUNDS,
            timeout,
        ),
        tx,
    )
}

#[test]
fn expired_approval_never_resumes_from_a_late_decision() {
    let (mut bridge, human) = configured(1, Duration::ZERO);
    let sent = forwarded(bridge.upstream(call(1)));
    forwarded_result(bridge.downstream(required(&sent["id"])));
    assert!(bridge.decisions().is_empty());
    assert_eq!(
        status(&mut bridge, &sent["id"])["result"]["structuredContent"]["status"],
        "expired"
    );
    assert!(
        matches!(bridge.upstream(call(2)), Upstream::Reply(_)),
        "a blocked presenter must still consume capacity"
    );
    human.send(Some(0)).unwrap();
    for _ in 0..20 {
        std::thread::sleep(Duration::from_millis(2));
        assert!(bridge.decisions().is_empty());
    }
    forwarded(bridge.upstream(call(2)));
}

#[test]
fn only_observed_completed_results_can_be_evicted() {
    let (mut bridge, human) = configured(1, Duration::from_secs(10));
    let sent = forwarded(bridge.upstream(call(1)));
    forwarded_result(bridge.downstream(required(&sent["id"])));
    assert!(matches!(
        bridge.downstream(required(&sent["id"])),
        Downstream::Consumed
    ));
    assert!(matches!(bridge.upstream(call(2)), Upstream::Reply(_)));
    human.send(Some(0)).unwrap();
    await_resume(&mut bridge);
    bridge.downstream(json!({"id":sent["id"],"result":{"content":[]}}));
    assert!(matches!(bridge.upstream(call(2)), Upstream::Reply(_)));
    status(&mut bridge, &sent["id"]);
    forwarded(bridge.upstream(call(2)));
    assert_eq!(
        status(&mut bridge, &sent["id"])["result"]["structuredContent"]["status"],
        "unknown"
    );
}

#[test]
fn listing_preserves_original_schema_and_does_not_duplicate_paginated_status() {
    let (mut bridge, _) = fixture();
    forwarded(bridge.upstream(json!({"id":4,"method":"tools/list"})));
    let schema = json!({"type":"object","required":["original"]});
    let response =
        forwarded_result(bridge.downstream(
            json!({"id":4,"result":{"tools":[{"name":"write","outputSchema":schema}]}}),
        ));
    assert_eq!(
        response["result"]["tools"][0]["outputSchema"]["anyOf"][0],
        schema
    );
    forwarded(bridge.upstream(json!({"id":5,"method":"tools/list","params":{"cursor":"page2"}})));
    let response = forwarded_result(bridge.downstream(json!({"id":5,"result":{"tools":[]}})));
    assert_eq!(response["result"]["tools"], json!([]));
}
fn call(id: i64) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":"fs.write","arguments":{"path":"original"}}})
}
fn forwarded(message: Upstream) -> Value {
    match message {
        Upstream::Forward(value) => value,
        _ => panic!("not forwarded"),
    }
}
fn forwarded_result(message: Downstream) -> Value {
    match message {
        Downstream::Forward(value) => value,
        _ => panic!("not forwarded"),
    }
}
fn required(id: &Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":{"resultType":"input_required","requestState":"opaque-runtime-state","inputRequests":{"consent":{"method":"elicitation/create","params":{"message":"Allow this operation?","requestedSchema":{"type":"object","properties":{"allow":{"type":"boolean"}},"required":["allow"]}}}}}})
}
fn status(bridge: &mut PendingApprovals, id: &Value) -> Value {
    match bridge.upstream(json!({"jsonrpc":"2.0","id":99,"method":"tools/call","params":{"name":STATUS_TOOL,"arguments":{"approval_id":id}}})) {
        Upstream::Reply(value) => value,
        _ => panic!("status must never reach runtime"),
    }
}
fn await_resume(bridge: &mut PendingApprovals) -> Value {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let mut messages = bridge.decisions();
        if !messages.is_empty() {
            assert_eq!(messages.len(), 1);
            return messages.remove(0);
        }
        assert!(
            Instant::now() < deadline,
            "human decision was not processed"
        );
        std::thread::yield_now();
    }
}
#[test]
fn pending_returns_before_human_and_status_never_reexecutes() {
    let (mut bridge, human) = fixture();
    let sent = forwarded(bridge.upstream(call(7)));
    assert_eq!(
        sent["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"],
        "2026-07-28"
    );
    assert_eq!(
        sent["params"]["_meta"]["io.modelcontextprotocol/clientCapabilities"],
        json!({"elicitation":{"form":{}}})
    );
    let reply = forwarded_result(bridge.downstream(required(&sent["id"])));
    assert_eq!(reply["id"], 7);
    assert_eq!(
        reply["result"]["structuredContent"]["status"],
        "awaiting_approval"
    );
    assert!(!reply.to_string().contains("opaque-runtime-state"));
    assert_eq!(reply["result"]["structuredContent"]["tool"], "fs.write");
    assert!(!reply.to_string().contains("\"path\""));
    for _ in 0..3 {
        assert_eq!(
            status(&mut bridge, &sent["id"])["result"]["structuredContent"]["status"],
            "awaiting_approval"
        );
        assert!(bridge.decisions().is_empty());
    }
    human.send(Some(0)).unwrap();
    let resume = await_resume(&mut bridge);
    assert_eq!(resume["id"], sent["id"]);
    assert_eq!(resume["params"]["name"], "fs.write");
    assert_eq!(
        resume["params"]["arguments"],
        call(7)["params"]["arguments"]
    );
    assert_eq!(resume["params"]["requestState"], "opaque-runtime-state");
    assert!(bridge.decisions().is_empty());
    assert!(matches!(bridge.downstream(json!({"jsonrpc":"2.0","id":sent["id"],"result":{"content":[{"type":"text","text":"written"}],"isError":false}})), Downstream::Consumed));
    for _ in 0..3 {
        let result = status(&mut bridge, &sent["id"]);
        assert_eq!(result["result"]["structuredContent"]["status"], "completed");
        assert_eq!(
            result["result"]["structuredContent"]["outcome"]["result"]["content"][0]["text"],
            "written"
        );
        assert!(bridge.decisions().is_empty());
    }
}
#[test]
fn reused_host_id_cannot_receive_an_old_operations_result() {
    let (mut bridge, human) = fixture();
    let first = forwarded(bridge.upstream(call(7)));
    let _ = forwarded_result(bridge.downstream(required(&first["id"])));
    let second = forwarded(bridge.upstream(call(7)));
    assert_ne!(first["id"], second["id"]);
    human.send(None).unwrap();
    let resume = await_resume(&mut bridge);
    assert_eq!(
        resume["params"]["inputResponses"]["consent"]["action"],
        "cancel"
    );
    assert!(matches!(bridge.downstream(json!({"id":first["id"],"result":{"isError":true,"content":[{"type":"text","text":"denied"}]}})), Downstream::Consumed));
    let response =
        forwarded_result(bridge.downstream(json!({"id":second["id"],"result":{"isError":false}})));
    assert_eq!(response["id"], 7);
    assert_eq!(
        status(&mut bridge, &first["id"])["result"]["structuredContent"]["outcome"]["result"]["isError"],
        true
    );
}
#[test]
fn status_cannot_submit_decisions_or_cross_sessions() {
    let (mut bridge, _) = fixture();
    let request = json!({"id":1,"method":"tools/call","params":{"name":STATUS_TOOL,"arguments":{"approval_id":"other-session","approve":true}}});
    let Upstream::Reply(reply) = bridge.upstream(request) else {
        panic!()
    };
    assert!(reply.get("error").is_some());
    assert_eq!(
        status(&mut bridge, &json!("other-session"))["result"]["structuredContent"]["status"],
        "unknown"
    );
}
#[test]
fn secret_forms_never_become_pending_approvals() {
    let (mut bridge, _) = fixture();
    let sent = forwarded(bridge.upstream(call(1)));
    let mut request = required(&sent["id"]);
    request["result"]["inputRequests"]["consent"]["params"]["requestedSchema"] = json!({"type":"object","properties":{"password":{"type":"string"}},"required":["password"]});
    let response = forwarded_result(bridge.downstream(request));
    assert_eq!(
        response["result"]["structuredContent"]["status"],
        "unavailable"
    );
    assert!(bridge.decisions().is_empty());
}
#[test]
fn tool_listing_advertises_read_only_status() {
    let (mut bridge, _) = fixture();
    forwarded(bridge.upstream(json!({"id":4,"method":"tools/list"})));
    let response = forwarded_result(bridge.downstream(json!({"id":4,"result":{"tools":[]}})));
    assert_eq!(response["result"]["tools"][0]["name"], STATUS_TOOL);
    assert_eq!(
        response["result"]["tools"][0]["annotations"]["readOnlyHint"],
        true
    );
}
