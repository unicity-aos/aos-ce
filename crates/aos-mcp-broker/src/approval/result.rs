//! Result delivery after a broker-owned approval binding.

use super::*;

/// Publish `decision` on the per-request response topic to unblock the host,
/// then drain the resumed/denied tool result and deliver it to the shim.
///
/// Subscribe-before-publish: the result subscription on
/// `tool.v1.execute.<tool_name>.result` is established BEFORE the decision is
/// published so the tool cannot resume and publish ahead of us. Then we drain
/// for `call_id` and reply on `astrid.v1.response.<req_id>` with the same
/// terminal `tool.call` shape the broker's non-parked path produces.
///
/// `tool_name` is charset-validated (it must form the result topic). A
/// non-routable `tool_name`, a subscribe failure, or a drain timeout still
/// publishes the decision (so the host-blocked tool is never left hanging)
/// and delivers a best-effort `isError` reply to the shim.
pub(super) fn resolve_with_decision(
    req: &ApprovalRespond,
    response_topic: &str,
    decision: &'static str,
    reason: Option<&str>,
    route: &PendingResultRoute,
) {
    let reply_topic = crate::broker::reply_topic(&req.req_id);

    // Validate the tool name BEFORE building the result topic — the same
    // charset gate the execute path applies, so a hostile / buggy echo
    // cannot smuggle topic segments into our subscription.
    if !crate::execute::is_valid_tool_name(&req.tool_name) {
        log::warn(format!(
            "{}: broker approval.respond: invalid tool_name '{}' for request_id '{}'; \
             publishing decision without result drain",
            crate::profile::log_tag(),
            req.tool_name,
            req.request_id
        ));
        // Still unblock the host so the tool retires, then surface an error.
        publish_decision(response_topic, &req.request_id, decision, reason);
        if let Some(reply) = &reply_topic {
            deliver_error(reply, &req.req_id, "approval echoed an invalid tool name");
        }
        return;
    }

    let result_topic = format!("tool.v1.execute.{}.result", req.tool_name);

    // Subscribe BEFORE publishing the decision so the resumed tool's result
    // cannot race ahead of our subscription. A subscribe failure is
    // non-fatal: we still publish the decision (the host must not be left
    // blocked) and fall back to a best-effort error reply.
    let result_sub = match ipc::subscribe(&result_topic) {
        Ok(s) => Some(s),
        Err(e) => {
            log::warn(format!(
                "{}: broker approval.respond: failed to subscribe {result_topic}: {e}",
                crate::profile::log_tag()
            ));
            None
        }
    };

    let principal = runtime::caller().ok().and_then(|caller| caller.principal);
    if route.delivery_id.is_some() && principal.as_deref().is_none_or(str::is_empty) {
        publish_decision(response_topic, &req.request_id, DENY, None);
        if let Some(reply) = &reply_topic {
            deliver_error(
                reply,
                &req.req_id,
                "resumed invocation has no authenticated principal",
            );
        }
        return;
    }
    let failure_sub = match route.delivery_id.as_deref() {
        Some(id) => match crate::execute::delivery_failure::subscribe(id) {
            Ok(sub) => Some(sub),
            Err(error) => {
                publish_decision(response_topic, &req.request_id, DENY, None);
                if let Some(reply) = &reply_topic {
                    deliver_error(
                        reply,
                        &req.req_id,
                        &format!("cannot observe resumed invocation: {error}"),
                    );
                }
                return;
            }
        },
        None => None, // Older persisted approvals retain their bounded timeout.
    };
    publish_decision(response_topic, &req.request_id, decision, reason);

    let Some(reply) = &reply_topic else {
        // Decision is published (tool unblocked) but we have no channel to
        // deliver the terminal result on. The shim will time out its own
        // request; nothing more we can do.
        log::warn(format!(
            "{}: broker approval.respond: decision published but req_id '{}' unroutable; \
             no terminal reply delivered",
            crate::profile::log_tag(),
            req.req_id
        ));
        return;
    };

    // Drain the resumed/denied result and deliver it as the terminal reply.
    match result_sub.and_then(|sub| {
        drain_result(
            &sub,
            &req.call_id,
            route,
            failure_sub.as_ref(),
            principal.as_deref(),
        )
    }) {
        Some((content, is_error)) => deliver_result(reply, &req.req_id, content, is_error),
        None => deliver_error(
            reply,
            &req.req_id,
            "tool did not return a result after the approval decision",
        ),
    }
}

/// Drain `tool.v1.execute.<name>.result` for `call_id` after a decision,
/// returning `(content, is_error)` for the first matching result or `None` on
/// timeout. Bounded by [`RESUME_TIMEOUT_MS`]; polled in [`RESUME_SLICE_MS`]
/// slices so a result published the instant the tool resumes is picked up
/// promptly. Reuses the execute path's result-envelope parser so both legs
/// agree on the wire shape.
fn drain_result(
    sub: &ipc::Subscription,
    call_id: &str,
    route: &PendingResultRoute,
    failure_sub: Option<&ipc::Subscription>,
    principal: Option<&str>,
) -> Option<(Value, bool)> {
    let mut remaining = RESUME_TIMEOUT_MS;
    while remaining > 0 {
        let step = remaining.min(RESUME_SLICE_MS);
        if let (Some(sub), Some(id), Some(principal)) =
            (failure_sub, route.delivery_id.as_deref(), principal)
            && let Some(reason) = crate::execute::delivery_failure::poll(
                sub,
                id,
                &format!("tool.v1.execute.{}", route.tool_name),
                principal,
            )
        {
            return Some((
                Value::String(format!("runtime invocation failed: {reason}")),
                true,
            ));
        }

        match sub.recv(step) {
            Ok(poll) => {
                for msg in poll.messages {
                    if let Some(found) = crate::execute::match_result(
                        &msg.payload,
                        call_id,
                        &msg.source_id,
                        &route.provider_source_id,
                    ) {
                        return Some(found);
                    }
                }
            }
            Err(_) => {
                // Slice timeout — keep draining until the budget closes.
            }
        }
        remaining = remaining.saturating_sub(step);
    }
    None
}
