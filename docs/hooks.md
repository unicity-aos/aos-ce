# Composable frontend hooks

AOS separates frontend protocol translation from canonical hook policy.

```text
frontend plugin
  -> authenticated ingress in capsule-mcp
  -> exact oracle.v1.hook.validated.<frontend> topic
  -> one frontend adapter
  -> canonical hook.v1.event.<semantic-hook>
  -> zero or more independent subscriber capsules
  -> optional correlation-scoped replies
  -> adapter-shaped frontend response
```

`aos-hook-bridge` is the parallel ingress for Astrid-native lifecycle events.
It does not parse Codex, Claude, or Grok protocols. This keeps the canonical
hook bus independent of any agent product and lets a new frontend ship as an
adapter capsule instead of changing the router or kernel.

## Trust boundaries

The frontend process is not trusted to name a principal. `capsule-mcp` verifies
the host-session token against the kernel-stamped invoking principal and
publishes a token-free validated envelope. The adapter then:

1. binds each exact validated topic to the expected frontend;
2. rejects a payload whose `host` does not match that topic;
3. checks route, delivery, correlation, size, and segment constraints;
4. checks the kernel-stamped caller principal against `principal_id`;
5. emits a bounded canonical event; and
6. accepts correlated replies only from the same verified principal.

The right to publish a validated topic is an install-reviewed IPC capability.
Only the authenticated ingress capsule should hold it. Adding a second raw
adapter to the same exact topic is a deployment error: raw protocol ingress has
one owner, while extension happens downstream on canonical hook topics.

## Response classes

Every mapping declares one response class.

### Observation

The adapter publishes without a correlation ID. Subscribers may observe but
cannot affect the frontend operation. Session, post-tool, compaction, sub-agent,
completed-response, and shutdown events use this class. Frontend adapters must
not conflate a per-turn response event with session termination:

- Codex and Claude `stop` map to `message_sent` and retain
  `last_assistant_message` in the nested frontend payload;
- Claude `message_display` maps to `message_displayed` and retains its streamed
  `delta`;
- Codex and Claude map only their explicit `session_end` event to canonical
  `session_end`; and
- Grok currently maps `stop` to canonical `session_end` because its installed
  hook contract does not expose a distinct verified termination event.

Adapter responses preserve both names: `event` is the source frontend event,
while `canonical_hook` is the semantic classification used by authenticated
route lifecycle handling. `capsule-mcp` retires a route only when
`canonical_hook` is `session_end`; during a staggered upgrade, a legacy response
without `canonical_hook` retires only on an explicit source `session_end`.

These hooks expose the submitted user prompt and rendered assistant text, not
the frontend's complete provider-bound prompt with hidden system, developer,
tool, or harness context.

`pre_tool_use` and `permission_request` are also observations on the current
Oracle relay because its outer response schema can return context only. Binding
native-tool denial stays on `astrid-gate` plus the broker policy responder,
which has a host-specific deny response. Treating an unrepresentable generic
reply as binding would be fail-open theater.

### Additional context

`user_prompt_submit` publishes `message_received` with the exact host
correlation ID. Subscribers may publish `{ "additional_context": "..." }` to
the response topic. The adapter accepts same-principal replies, combines them
within 64 KiB, and drops the entire partial result if the response subscription
reports lag or loss.

### Binding lifecycle result

Astrid-native lifecycle events enter `aos-hook-bridge`. It creates its own
correlation ID and returns the merged result to the invoking lifecycle
interceptor:

- `before_tool_call`: any `skip: true` wins; the last non-null
  `modified_params` wins;
- `after_tool_call`: last non-null `modified_result` wins;
- `tool_result_persist`: last non-null `transformed_result` wins; and
- `message_sending`: last non-null `modified_content` wins.

Replies are accepted only on the exact correlation topic and from the
kernel-verified invoking principal. Known fan-out loss, lag, route mismatch,
principal mismatch, oversize, or malformed JSON invalidates the whole batch:
`before_tool_call` returns `skip: true`, while transformation hooks discard all
partial output.

A timeout with no reply still means "no mutation requested" because the bus
does not currently expose an expected-responder registry. Therefore this
fan-out is not, by itself, proof that every required policy capsule approved.
Mandatory native-tool security remains the direct `astrid-gate` and broker
path. A future mandatory lifecycle policy needs explicit required-responder
registration rather than inferring approval from silence.

These merge rules are protocol policy. A future frontend may bind to them only
after its own adapter can faithfully translate the merged result into that
frontend's native response schema. That change needs per-frontend conformance
tests for deny, allow/no-op, malformed response, timeout, and transport error.

## Priority and layering

Do not assign priorities to observation subscribers. Equal default priority
keeps independent concurrent fan-out: every subscriber sees the original event
and one subscriber cannot suppress another.

Do not use priority to order frontend adapters. Each authenticated raw topic has
one adapter owner.

For an intentionally binding middleware topic, the following bands are
reserved guidance:

| Band | Purpose | Required behavior |
|---|---|---|
| 10-19 | validate and normalize | malformed input returns `Deny`, not `Err`, when safety depends on rejection |
| 20-39 | safety and policy narrowing | deny may stop the chain; no layer may broaden prior authority |
| 40-79 | deterministic transformation | transformed payload is explicit and tested |
| 80-99 | enrichment | must not reinterpret a deny as allow |
| 100+ | provider/application execution | receives only the prior stage's accepted payload |

Setting one distinct priority converts **all** matching handlers from fan-out
to one ordered chain. Therefore a capsule must not introduce a priority on an
existing canonical topic unless that topic is explicitly declared binding and
every matching capsule is reviewed as middleware. Ordinary handler errors
continue in an ordered chain; a security layer must return `Deny` for a refusal.

## Adding a frontend

1. Add authenticated ingress and a frontend-specific validated topic.
2. Add one adapter capsule or one isolated handler table with exact topic/host
   binding.
3. Map only events whose semantics are genuinely equivalent.
4. Classify every mapping as observation, context, or binding.
5. Keep the original payload nested under canonical provenance rather than
   flattening untrusted keys into the envelope.
6. Bound input, canonical output, correlation identifiers, replies, and wait
   time.
7. Add negative tests for cross-topic host claims, principal mismatch, topic
   smuggling, stale routes, oversized payloads, reply loss, and unsupported
   events.
8. Do not change the kernel. Translation and policy belong in capsules.
