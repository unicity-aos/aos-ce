use super::*;

#[test]
fn stale_idle_generation_reaches_redrive_but_active_generation_rejects_it() {
    let old = Uuid::new_v4();
    let next = Uuid::new_v4();
    let reply = serde_json::json!({"request_id": next.to_string()});
    let mut state = TurnState {
        phase: Phase::Idle,
        request_id: old,
        ..TurnState::default()
    };
    assert!(!orchestration_reply_is_stale(&state, &reply));
    for phase in [
        Phase::AwaitingIdentity,
        Phase::AwaitingPromptBuild,
        Phase::Streaming,
        Phase::AwaitingTools,
    ] {
        state.phase = phase;
        assert!(orchestration_reply_is_stale(&state, &reply));
        state.request_id = next;
        assert!(!orchestration_reply_is_stale(&state, &reply));
        assert!(orchestration_reply_is_stale(&state, &serde_json::json!({})));
        state.request_id = old;
    }
}
