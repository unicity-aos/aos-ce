//! Bounded native correlation for modern MCP `input_required` results.
//!
//! Remembers in-flight `tools/call` invocations by JSON-RPC id and turns a
//! matching `InputRequiredResult` into a resume `tools/call` that echoes
//! `requestState` opaquely. Presence in the table means the call is still open.
//! The token is never authority to change the original method, name, arguments,
//! or `_meta`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde_json::{Map, Value, json};

use super::interaction::{self, Presenter};

const TOOLS_CALL: &str = "tools/call";
const ELICITATION_CREATE: &str = "elicitation/create";
const RESULT_TYPE_INPUT_REQUIRED: &str = "input_required";
const CANCELLED_NOTIFICATION: &str = "notifications/cancelled";
const MAX_INPUT_REQUESTS: usize = 8;
/// One MCP serve process is one host session. Parallel tool calls fit here;
/// each entry retains cloned `tools/call` params, so this is also a memory bound.
pub(super) const DEFAULT_MAX_IN_FLIGHT_CALLS: usize = 32;
pub(super) const MAX_MAX_IN_FLIGHT_CALLS: usize = 1024;
/// Distinct subsequent rounds remain possible; unbounded `requestState` retention does not.
pub(super) const DEFAULT_MAX_INPUT_ROUNDS: usize = 8;
pub(super) const MAX_MAX_INPUT_ROUNDS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum RpcId {
    Number(i64),
    String(String),
}

impl RpcId {
    fn parse(value: &Value) -> Result<Self, MrtrError> {
        match value {
            Value::String(value) => Ok(Self::String(value.clone())),
            Value::Number(number) => number
                .as_i64()
                .map(Self::Number)
                .ok_or(MrtrError::Malformed(
                    "JSON-RPC id must be a string or integer",
                )),
            _ => Err(MrtrError::Malformed(
                "JSON-RPC id must be a string or integer",
            )),
        }
    }
}

#[derive(Debug, Clone)]
struct TrackedCall {
    jsonrpc: Value,
    id: Value,
    method: String,
    params: Map<String, Value>,
    consumed_states: BTreeSet<String>,
    consumed_stateless: bool,
}

/// In-memory table of native MRTR invocations awaiting a local decision.
#[derive(Debug)]
pub(super) struct NativeMrtr {
    calls: BTreeMap<RpcId, TrackedCall>,
    max_in_flight: usize,
    max_input_rounds: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum MrtrError {
    UnknownId,
    AlreadySettled,
    DuplicateId,
    TooManyInFlight,
    TooManyRounds,
    Malformed(&'static str),
    Unsupported(&'static str),
}

impl fmt::Display for MrtrError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownId => formatter.write_str("input_required id is not a tracked tools/call"),
            Self::AlreadySettled => {
                formatter.write_str("tracked tools/call already consumed this input_required round")
            }
            Self::DuplicateId => formatter.write_str("tools/call id is already in flight"),
            Self::TooManyInFlight => {
                formatter.write_str("too many in-flight native tools/call invocations")
            }
            Self::TooManyRounds => {
                formatter.write_str("tracked tools/call exceeded the native input round limit")
            }
            Self::Malformed(message) | Self::Unsupported(message) => formatter.write_str(message),
        }
    }
}

struct Prepared {
    key: RpcId,
    request_state: Option<String>,
    input_requests: Option<Map<String, Value>>,
}

impl NativeMrtr {
    #[cfg(test)]
    pub(super) fn new() -> Self {
        Self::with_limits(DEFAULT_MAX_IN_FLIGHT_CALLS, DEFAULT_MAX_INPUT_ROUNDS)
    }

    pub(super) fn with_limits(max_in_flight: usize, max_input_rounds: usize) -> Self {
        Self {
            calls: BTreeMap::new(),
            max_in_flight,
            max_input_rounds,
        }
    }

    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.calls.is_empty()
    }

    /// Remember a host `tools/call` so a later `input_required` can resume it.
    ///
    /// Returns `true` when the request is tracked. Other methods are ignored.
    pub(super) fn record(&mut self, request: &Value) -> Result<bool, MrtrError> {
        if request.get("method").and_then(Value::as_str) != Some(TOOLS_CALL) {
            return Ok(false);
        }
        let id = request
            .get("id")
            .cloned()
            .ok_or(MrtrError::Malformed("tools/call has no id"))?;
        let key = RpcId::parse(&id)?;
        let params = request
            .get("params")
            .and_then(Value::as_object)
            .ok_or(MrtrError::Malformed("tools/call params must be an object"))?;
        if params.get("name").and_then(Value::as_str).is_none() {
            return Err(MrtrError::Malformed("tools/call has no tool name"));
        }
        if self.calls.contains_key(&key) {
            return Err(MrtrError::DuplicateId);
        }
        if self.calls.len() >= self.max_in_flight {
            return Err(MrtrError::TooManyInFlight);
        }

        let mut stored = params.clone();
        stored.remove("inputResponses");
        stored.remove("requestState");
        self.calls.insert(
            key,
            TrackedCall {
                jsonrpc: request
                    .get("jsonrpc")
                    .cloned()
                    .unwrap_or_else(|| Value::String("2.0".to_owned())),
                id,
                method: TOOLS_CALL.to_owned(),
                params: stored,
                consumed_states: BTreeSet::new(),
                consumed_stateless: false,
            },
        );
        Ok(true)
    }

    /// Drop a tracked call after a terminal result, cancellation, or EOF.
    pub(super) fn complete(&mut self, id: &Value) -> bool {
        RpcId::parse(id)
            .ok()
            .is_some_and(|key| self.calls.remove(&key).is_some())
    }

    /// Drop tracking for a cancelled request id without resuming it.
    pub(super) fn forget(&mut self, id: &Value) {
        let _ = self.complete(id);
    }

    /// Turn a correlated `input_required` result into a resume `tools/call`.
    ///
    /// Forms are parsed before the presenter runs. Unsupported, malformed, or
    /// presenter-failed input fail closed with cancel responses and no consent.
    /// A successful resume consumes that round's `requestState` so the same
    /// round cannot be decided twice, but a later distinct round may continue.
    pub(super) fn decide(
        &mut self,
        result_message: &Value,
        presenter: &mut dyn Presenter,
    ) -> Result<Value, MrtrError> {
        let prepared = self.prepare(result_message)?;
        let responses = match collect_elicitations(prepared.input_requests.as_ref()) {
            Ok(envelopes) => match present_all(&envelopes, presenter) {
                Ok(responses) => responses,
                Err(_) => cancel_all(prepared.input_requests.as_ref()),
            },
            Err(_) => cancel_all(prepared.input_requests.as_ref()),
        };
        self.finish_resume(prepared, responses)
    }

    /// Fail closed without opening a presenter: cancel each form and echo state.
    pub(super) fn decline(&mut self, result_message: &Value) -> Result<Value, MrtrError> {
        let prepared = self.prepare(result_message)?;
        let responses = cancel_all(prepared.input_requests.as_ref());
        self.finish_resume(prepared, responses)
    }

    fn prepare(&self, result_message: &Value) -> Result<Prepared, MrtrError> {
        if result_message.get("method").is_some() {
            return Err(MrtrError::Malformed(
                "input_required must be a JSON-RPC result, not a request",
            ));
        }
        if result_message.get("error").is_some() {
            return Err(MrtrError::Malformed("JSON-RPC error is not input_required"));
        }
        let id = result_message
            .get("id")
            .cloned()
            .ok_or(MrtrError::Malformed("input_required result has no id"))?;
        let key = RpcId::parse(&id)?;
        let Some(call) = self.calls.get(&key) else {
            return Err(MrtrError::UnknownId);
        };

        let result = result_message
            .get("result")
            .and_then(Value::as_object)
            .ok_or(MrtrError::Malformed(
                "input_required result must be an object",
            ))?;
        if result.get("resultType").and_then(Value::as_str) != Some(RESULT_TYPE_INPUT_REQUIRED) {
            return Err(MrtrError::Malformed(
                "InputRequiredResult requires resultType to be \"input_required\"",
            ));
        }

        let request_state = match result.get("requestState") {
            None => None,
            Some(Value::String(state)) => Some(state.clone()),
            Some(_) => {
                return Err(MrtrError::Malformed("requestState must be a string"));
            }
        };
        let input_requests = match result.get("inputRequests") {
            None => None,
            Some(Value::Object(map)) => Some(map.clone()),
            Some(_) => {
                return Err(MrtrError::Malformed("inputRequests must be an object"));
            }
        };
        if request_state.is_none() && input_requests.is_none() {
            return Err(MrtrError::Malformed(
                "InputRequiredResult requires at least one of inputRequests or requestState",
            ));
        }
        if round_already_consumed(call, request_state.as_deref()) {
            return Err(MrtrError::AlreadySettled);
        }
        if retained_rounds(call) >= self.max_input_rounds {
            return Err(MrtrError::TooManyRounds);
        }
        Ok(Prepared {
            key,
            request_state,
            input_requests,
        })
    }

    fn finish_resume(
        &mut self,
        prepared: Prepared,
        responses: Map<String, Value>,
    ) -> Result<Value, MrtrError> {
        let call = self
            .calls
            .get_mut(&prepared.key)
            .ok_or(MrtrError::UnknownId)?;
        if round_already_consumed(call, prepared.request_state.as_deref()) {
            return Err(MrtrError::AlreadySettled);
        }
        if retained_rounds(call) >= self.max_input_rounds {
            return Err(MrtrError::TooManyRounds);
        }
        match prepared.request_state.as_ref() {
            Some(state) => {
                call.consumed_states.insert(state.clone());
            }
            None => call.consumed_stateless = true,
        }
        let mut params = call.params.clone();
        if !responses.is_empty() {
            params.insert("inputResponses".to_owned(), Value::Object(responses));
        }
        if let Some(request_state) = prepared.request_state {
            params.insert("requestState".to_owned(), Value::String(request_state));
        }
        Ok(json!({
            "jsonrpc": call.jsonrpc.clone(),
            "id": call.id.clone(),
            "method": call.method.clone(),
            "params": params,
        }))
    }
}

pub(super) fn is_input_required_result(message: &Value) -> bool {
    message.get("method").is_none()
        && message.get("error").is_none()
        && message
            .get("result")
            .and_then(|result| result.get("resultType"))
            .and_then(Value::as_str)
            == Some(RESULT_TYPE_INPUT_REQUIRED)
}

pub(super) fn cancelled_request_id(message: &Value) -> Option<&Value> {
    if message.get("method").and_then(Value::as_str) != Some(CANCELLED_NOTIFICATION) {
        return None;
    }
    message.get("params")?.get("requestId")
}

fn round_already_consumed(call: &TrackedCall, request_state: Option<&str>) -> bool {
    match request_state {
        Some(state) => call.consumed_states.contains(state),
        None => call.consumed_stateless,
    }
}

fn retained_rounds(call: &TrackedCall) -> usize {
    call.consumed_states
        .len()
        .saturating_add(usize::from(call.consumed_stateless))
}

pub(super) fn parse_max_in_flight_calls(value: &str) -> Result<usize, String> {
    parse_limit(value, 1, MAX_MAX_IN_FLIGHT_CALLS, "max-in-flight-calls")
}

pub(super) fn parse_max_input_rounds(value: &str) -> Result<usize, String> {
    parse_limit(value, 1, MAX_MAX_INPUT_ROUNDS, "max-input-rounds")
}

fn parse_limit(value: &str, min: usize, max: usize, name: &str) -> Result<usize, String> {
    let parsed = value
        .parse::<usize>()
        .map_err(|_| format!("{name} must be an integer"))?;
    if !(min..=max).contains(&parsed) {
        return Err(format!("{name} must be between {min} and {max}"));
    }
    Ok(parsed)
}

fn present_all(
    envelopes: &[(String, Value)],
    presenter: &mut dyn Presenter,
) -> Result<Map<String, Value>, MrtrError> {
    let mut responses = Map::new();
    for (key, envelope) in envelopes {
        let resolved = interaction::resolve(envelope, presenter).map_err(|error| match error {
            interaction::InteractionError::Unsupported(_) => {
                MrtrError::Unsupported("local interaction refused the form")
            }
            interaction::InteractionError::Invalid(_) => {
                MrtrError::Malformed("elicitation form is malformed")
            }
            interaction::InteractionError::Unavailable(_) => {
                MrtrError::Unsupported("local interaction provider failed")
            }
        })?;
        let decision = resolved
            .get("result")
            .cloned()
            .ok_or(MrtrError::Malformed("local interaction returned no result"))?;
        responses.insert(key.clone(), decision);
    }
    Ok(responses)
}

fn cancel_all(input_requests: Option<&Map<String, Value>>) -> Map<String, Value> {
    let mut responses = Map::new();
    if let Some(map) = input_requests {
        for key in map.keys() {
            responses.insert(key.clone(), json!({ "action": "cancel" }));
        }
    }
    responses
}

fn collect_elicitations(
    input_requests: Option<&Map<String, Value>>,
) -> Result<Vec<(String, Value)>, MrtrError> {
    let Some(input_requests) = input_requests else {
        return Ok(Vec::new());
    };
    if input_requests.len() > MAX_INPUT_REQUESTS {
        return Err(MrtrError::Unsupported(
            "input_required asked for more forms than the native bridge will collect",
        ));
    }
    let mut envelopes = Vec::with_capacity(input_requests.len());
    for (key, request) in input_requests {
        let envelope = elicitation_envelope(key, request)?;
        interaction::parse_request(&envelope).map_err(|error| match error {
            interaction::InteractionError::Unsupported(_) => {
                MrtrError::Unsupported("local interaction refused the form")
            }
            interaction::InteractionError::Invalid(_) => {
                MrtrError::Malformed("elicitation form is malformed")
            }
            interaction::InteractionError::Unavailable(_) => {
                MrtrError::Malformed("elicitation form is malformed")
            }
        })?;
        envelopes.push((key.clone(), envelope));
    }
    Ok(envelopes)
}

fn elicitation_envelope(key: &str, request: &Value) -> Result<Value, MrtrError> {
    let object = request
        .as_object()
        .ok_or(MrtrError::Malformed("input request must be an object"))?;
    match object.get("method").and_then(Value::as_str) {
        Some(ELICITATION_CREATE) => {}
        Some("sampling/createMessage") | Some("roots/list") => {
            return Err(MrtrError::Unsupported(
                "native consent only answers elicitation/create input requests",
            ));
        }
        Some(_) => {
            return Err(MrtrError::Unsupported(
                "native consent refused an unknown input request method",
            ));
        }
        None => {
            return Err(MrtrError::Malformed("input request has no method"));
        }
    }
    let params = object.get("params").cloned().ok_or(MrtrError::Malformed(
        "elicitation input request has no params",
    ))?;
    if !params.is_object() {
        return Err(MrtrError::Malformed(
            "elicitation input request params must be an object",
        ));
    }
    Ok(json!({
        "jsonrpc": "2.0",
        "id": key,
        "method": ELICITATION_CREATE,
        "params": params,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct FakePresenter {
        choice: Option<usize>,
        calls: usize,
        fail: bool,
    }

    impl FakePresenter {
        fn accept(choice: usize) -> Self {
            Self {
                choice: Some(choice),
                calls: 0,
                fail: false,
            }
        }

        fn unused() -> Self {
            Self {
                choice: Some(0),
                calls: 0,
                fail: false,
            }
        }

        fn failing() -> Self {
            Self {
                choice: Some(0),
                calls: 0,
                fail: true,
            }
        }
    }

    impl Presenter for FakePresenter {
        fn present(
            &mut self,
            _request: &interaction::InteractionRequest,
        ) -> Result<Option<usize>, interaction::InteractionError> {
            self.calls += 1;
            if self.fail {
                return Err(interaction::InteractionError::Unavailable(
                    "presenter exploded".to_owned(),
                ));
            }
            Ok(self.choice)
        }
    }

    fn tool_call() -> Value {
        tool_call_with_id(json!(7))
    }

    fn tool_call_with_id(id: Value) -> Value {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {
                "_meta": {
                    "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                    "io.modelcontextprotocol/clientCapabilities": {
                        "elicitation": { "form": {} }
                    }
                },
                "name": "fs.read",
                "arguments": { "path": "/tmp/report" },
                "extra": "keep-me"
            }
        })
    }

    fn grant_schema() -> Value {
        json!({
            "type": "object",
            "properties": { "grant": { "type": "boolean" } },
            "required": ["grant"]
        })
    }

    fn input_required(id: Value, state: &str, schema: Value) -> Value {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "resultType": "input_required",
                "requestState": state,
                "inputRequests": {
                    "astrid-consent": {
                        "method": "elicitation/create",
                        "params": {
                            "mode": "form",
                            "message": "Allow this capsule to continue?",
                            "requestedSchema": schema
                        }
                    }
                }
            }
        })
    }

    #[test]
    fn accept_preserves_original_call_and_echoes_opaque_state() {
        let mut session = NativeMrtr::new();
        assert!(session.record(&tool_call()).expect("record"));
        let poison = r#"{"name":"fs.write","arguments":{"path":"/etc/passwd"}}"#;
        let mut presenter = FakePresenter::accept(0);
        let resume = session
            .decide(
                &input_required(json!(7), poison, grant_schema()),
                &mut presenter,
            )
            .expect("decide");

        assert_eq!(presenter.calls, 1);
        assert_eq!(resume["method"], TOOLS_CALL);
        assert_eq!(resume["id"], 7);
        assert_eq!(resume["params"]["name"], "fs.read");
        assert_eq!(
            resume["params"]["arguments"],
            json!({ "path": "/tmp/report" })
        );
        assert_eq!(
            resume["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"],
            "2026-07-28"
        );
        assert_eq!(resume["params"]["extra"], "keep-me");
        assert_eq!(resume["params"]["requestState"], poison);
        assert_eq!(
            resume["params"]["inputResponses"]["astrid-consent"],
            json!({ "action": "accept", "content": { "grant": true } })
        );
        assert!(!session.is_empty());
    }

    #[test]
    fn unknown_id_is_rejected_without_presenter() {
        let mut session = NativeMrtr::new();
        session.record(&tool_call()).expect("record");
        let mut presenter = FakePresenter::unused();
        let error = session
            .decide(
                &input_required(json!("missing"), "state", grant_schema()),
                &mut presenter,
            )
            .expect_err("unknown id");
        assert_eq!(error, MrtrError::UnknownId);
        assert_eq!(presenter.calls, 0);
    }

    #[test]
    fn completed_call_is_released_and_not_replayed() {
        let mut session = NativeMrtr::new();
        session.record(&tool_call()).expect("record");
        assert!(session.complete(&json!(7)));
        assert!(session.is_empty());
        let mut presenter = FakePresenter::unused();
        let error = session
            .decide(
                &input_required(json!(7), "state", grant_schema()),
                &mut presenter,
            )
            .expect_err("replay");
        assert_eq!(error, MrtrError::UnknownId);
        assert_eq!(presenter.calls, 0);
    }

    #[test]
    fn successful_round_cannot_be_replayed_but_later_state_may_continue() {
        let mut session = NativeMrtr::new();
        session.record(&tool_call()).expect("record");
        let mut presenter = FakePresenter::accept(0);
        session
            .decide(
                &input_required(json!(7), "once", grant_schema()),
                &mut presenter,
            )
            .expect("first decide");
        let error = session
            .decide(
                &input_required(json!(7), "once", grant_schema()),
                &mut presenter,
            )
            .expect_err("duplicate round");
        assert_eq!(error, MrtrError::AlreadySettled);
        assert_eq!(presenter.calls, 1);
        let resume = session
            .decide(
                &input_required(json!(7), "later", grant_schema()),
                &mut presenter,
            )
            .expect("later round");
        assert_eq!(presenter.calls, 2);
        assert_eq!(resume["params"]["name"], "fs.read");
        assert_eq!(resume["params"]["requestState"], "later");
        assert_eq!(
            resume["params"]["arguments"],
            json!({ "path": "/tmp/report" })
        );
        assert!(session.complete(&json!(7)));
        assert!(session.is_empty());
    }

    #[test]
    fn password_form_fails_closed_without_presenter() {
        let mut session = NativeMrtr::new();
        session.record(&tool_call()).expect("record");
        let mut presenter = FakePresenter::unused();
        let resume = session
            .decide(
                &input_required(
                    json!(7),
                    "secret-state",
                    json!({
                        "type": "object",
                        "properties": {
                            "secret": { "type": "string", "format": "password" }
                        }
                    }),
                ),
                &mut presenter,
            )
            .expect("fail closed");
        assert_eq!(presenter.calls, 0);
        assert_eq!(resume["params"]["name"], "fs.read");
        assert_eq!(resume["params"]["requestState"], "secret-state");
        assert_eq!(
            resume["params"]["inputResponses"]["astrid-consent"],
            json!({ "action": "cancel" })
        );
        let replay = session
            .decide(
                &input_required(json!(7), "secret-state", grant_schema()),
                &mut presenter,
            )
            .expect_err("consumed after fail-closed");
        assert_eq!(replay, MrtrError::AlreadySettled);
    }

    #[test]
    fn sampling_input_fails_closed_without_presenter() {
        let mut session = NativeMrtr::new();
        session.record(&tool_call()).expect("record");
        let mut presenter = FakePresenter::unused();
        let result = json!({
            "jsonrpc": "2.0",
            "id": 7,
            "result": {
                "resultType": "input_required",
                "requestState": "state",
                "inputRequests": {
                    "ask": {
                        "method": "sampling/createMessage",
                        "params": { "messages": [], "maxTokens": 16 }
                    }
                }
            }
        });
        let resume = session
            .decide(&result, &mut presenter)
            .expect("fail closed");
        assert_eq!(presenter.calls, 0);
        assert_eq!(resume["params"]["requestState"], "state");
        assert_eq!(
            resume["params"]["inputResponses"]["ask"],
            json!({ "action": "cancel" })
        );
    }

    #[test]
    fn decline_echoes_state_without_ui() {
        let mut session = NativeMrtr::new();
        session.record(&tool_call()).expect("record");
        let resume = session
            .decline(&input_required(json!(7), "opaque", grant_schema()))
            .expect("decline");
        assert_eq!(resume["params"]["name"], "fs.read");
        assert_eq!(resume["params"]["requestState"], "opaque");
        assert_eq!(
            resume["params"]["inputResponses"]["astrid-consent"],
            json!({ "action": "cancel" })
        );
        assert!(!session.is_empty());
    }

    #[test]
    fn state_only_resume_keeps_original_args() {
        let mut session = NativeMrtr::new();
        session.record(&tool_call()).expect("record");
        let mut presenter = FakePresenter::unused();
        let resume = session
            .decide(
                &json!({
                    "jsonrpc": "2.0",
                    "id": 7,
                    "result": {
                        "resultType": "input_required",
                        "requestState": "shed"
                    }
                }),
                &mut presenter,
            )
            .expect("state-only");
        assert_eq!(presenter.calls, 0);
        assert_eq!(resume["params"]["name"], "fs.read");
        assert_eq!(resume["params"]["arguments"]["path"], "/tmp/report");
        assert_eq!(resume["params"]["requestState"], "shed");
        assert!(resume["params"].get("inputResponses").is_none());
    }

    #[test]
    fn missing_result_type_is_malformed() {
        let mut session = NativeMrtr::new();
        session.record(&tool_call()).expect("record");
        let mut presenter = FakePresenter::unused();
        let error = session
            .decide(
                &json!({
                    "id": 7,
                    "result": { "requestState": "state" }
                }),
                &mut presenter,
            )
            .expect_err("missing resultType");
        assert!(matches!(error, MrtrError::Malformed(_)));
        assert_eq!(presenter.calls, 0);
        session
            .decide(
                &input_required(json!(7), "state", grant_schema()),
                &mut presenter,
            )
            .expect("a non-input_required message must not consume the tracked call");
        assert_eq!(presenter.calls, 1);
    }

    #[test]
    fn presenter_error_fail_closes_without_leaving_the_round_open() {
        let mut session = NativeMrtr::new();
        session.record(&tool_call()).expect("record");
        let mut presenter = FakePresenter::failing();
        let resume = session
            .decide(
                &input_required(json!(7), "boom", grant_schema()),
                &mut presenter,
            )
            .expect("fail closed");
        assert_eq!(presenter.calls, 1);
        assert_eq!(
            resume["params"]["inputResponses"]["astrid-consent"],
            json!({ "action": "cancel" })
        );
        let mut unused = FakePresenter::unused();
        let replay = session
            .decide(
                &input_required(json!(7), "boom", grant_schema()),
                &mut unused,
            )
            .expect_err("consumed after presenter error");
        assert_eq!(replay, MrtrError::AlreadySettled);
        assert_eq!(unused.calls, 0);
    }

    #[test]
    fn jsonrpc_ids_must_be_string_or_integer() {
        let mut session = NativeMrtr::new();
        for id in [json!(true), json!(null), json!([]), json!({}), json!(1.5)] {
            let mut request = tool_call();
            request["id"] = id.clone();
            let error = session.record(&request).expect_err("invalid id");
            assert!(
                matches!(error, MrtrError::Malformed(_)),
                "id {id} must be rejected"
            );
        }
        assert!(session.is_empty());

        let mut string_call = tool_call();
        string_call["id"] = json!("call-7");
        assert!(session.record(&string_call).expect("string id"));
        let mut presenter = FakePresenter::accept(0);
        let resume = session
            .decide(
                &input_required(json!("call-7"), "s", grant_schema()),
                &mut presenter,
            )
            .expect("string decide");
        assert_eq!(resume["id"], "call-7");
        assert_eq!(resume["params"]["name"], "fs.read");
    }

    #[test]
    fn cancellation_drops_tracking_without_resume() {
        let mut session = NativeMrtr::new();
        session.record(&tool_call()).expect("record");
        session.forget(&json!(7));
        assert!(session.is_empty());
        let mut presenter = FakePresenter::unused();
        let error = session
            .decide(
                &input_required(json!(7), "state", grant_schema()),
                &mut presenter,
            )
            .expect_err("cancelled");
        assert_eq!(error, MrtrError::UnknownId);
        assert_eq!(presenter.calls, 0);
    }

    #[test]
    fn excess_in_flight_call_is_rejected_without_dropping_existing() {
        let mut session = NativeMrtr::with_limits(2, DEFAULT_MAX_INPUT_ROUNDS);
        assert!(session.record(&tool_call_with_id(json!(1))).expect("first"));
        assert!(
            session
                .record(&tool_call_with_id(json!(2)))
                .expect("second")
        );
        assert_eq!(
            session
                .record(&tool_call_with_id(json!(3)))
                .expect_err("limit+1"),
            MrtrError::TooManyInFlight
        );
        assert_eq!(
            session
                .record(&tool_call_with_id(json!(2)))
                .expect_err("duplicate still wins"),
            MrtrError::DuplicateId
        );

        let resume = session
            .decline(&input_required(json!(1), "keep-first", grant_schema()))
            .expect("existing call remains");
        assert_eq!(resume["params"]["name"], "fs.read");
        assert_eq!(resume["params"]["requestState"], "keep-first");
        assert!(session.complete(&json!(2)));
        assert!(
            session
                .record(&tool_call_with_id(json!(3)))
                .expect("cleanup frees a slot")
        );
        assert_eq!(
            session
                .record(&tool_call_with_id(json!(4)))
                .expect_err("full again"),
            MrtrError::TooManyInFlight
        );
    }

    #[test]
    fn excess_distinct_states_are_rejected_without_dropping_the_call() {
        let mut session = NativeMrtr::with_limits(DEFAULT_MAX_IN_FLIGHT_CALLS, 2);
        session.record(&tool_call()).expect("record");
        let mut presenter = FakePresenter::accept(0);
        session
            .decide(
                &input_required(json!(7), "round-a", grant_schema()),
                &mut presenter,
            )
            .expect("first round");
        session
            .decide(
                &input_required(json!(7), "round-b", grant_schema()),
                &mut presenter,
            )
            .expect("second round");
        assert_eq!(presenter.calls, 2);
        let mut unused = FakePresenter::unused();
        let error = session
            .decide(
                &input_required(json!(7), "round-c", grant_schema()),
                &mut unused,
            )
            .expect_err("limit+1 round");
        assert_eq!(error, MrtrError::TooManyRounds);
        assert_eq!(unused.calls, 0);
        let replay = session
            .decide(
                &input_required(json!(7), "round-a", grant_schema()),
                &mut unused,
            )
            .expect_err("earlier round still consumed");
        assert_eq!(replay, MrtrError::AlreadySettled);
        assert!(!session.is_empty());
        assert!(session.complete(&json!(7)));
        assert!(session.is_empty());
        session
            .record(&tool_call())
            .expect("cleanup starts a new call");
        session
            .decide(
                &input_required(json!(7), "round-c", grant_schema()),
                &mut presenter,
            )
            .expect("new call is not bound by the previous round set");
        assert_eq!(presenter.calls, 3);
    }

    #[test]
    fn in_flight_and_round_limits_reject_zero_and_overflow() {
        assert!(parse_max_in_flight_calls("0").is_err());
        assert!(parse_max_in_flight_calls("1025").is_err());
        assert!(parse_max_in_flight_calls("nope").is_err());
        assert_eq!(parse_max_in_flight_calls("1").expect("min"), 1);
        assert_eq!(
            parse_max_in_flight_calls("1024").expect("max"),
            MAX_MAX_IN_FLIGHT_CALLS
        );
        assert!(parse_max_input_rounds("0").is_err());
        assert!(parse_max_input_rounds("65").is_err());
        assert_eq!(parse_max_input_rounds("8").expect("default"), 8);
    }
}
