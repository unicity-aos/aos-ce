# aos-hook-adapter-oracle

Translates authenticated Codex, Claude, and Grok frontend hook envelopes into
the canonical AOS `hook.v1.event.*` protocol.

The adapter owns protocol translation only. It does not own downstream hook
policy and it does not turn observation events into authorization decisions.
`user_prompt_submit`, `pre_tool_use`, and supported `permission_request` events
collect authenticated, bounded policy replies. Required responders are named
by `AOS_ORACLE_REQUIRED_HOOK_SOURCES`; missing or malformed required replies are
not approval. Oracle translates the neutral verdict to each client's format.

## One handler for all clients

All three frontend `PreToolUse` events publish the same
`hook.v1.event.before_tool_call` topic. One policy capsule subscribes to that
topic; it does not need separate Claude, Codex, and Grok handlers. Respond on
`hook.v1.response.before_tool_call.<correlation_id>` with the existing neutral
decision (`skip`, `ask`, `reason`). No objection leaves native permissions in
control; it does not grant an explicit allow.

The message is a `HookEventRequest` with `hook`, a JSON-string `payload`, and
`correlation_id`. The decoded payload contains authenticated `principal_id`
and `session_id`, frontend provenance (`host`, `source_event`), optional turn
and workspace identity, and the native `payload` object. Common tool fields in
that object use consistent snake-case names: `tool_name`, `tool_input`,
`tool_use_id`, `tool_response`, `permission_mode`, and `tool_input_truncated`.
Native aliases and extension fields remain intact for compatibility. Conflicting
aliases are rejected rather than choosing whichever representation satisfies
a policy. Tool names themselves are not guessed or renamed: different clients
may expose genuinely different tools.

This is a shared protocol, not a global permission scope. Replies must retain
the requesting principal and correlation; a handler cannot approve a different
user's request. Client-specific hooks can retain their distinct meaning on
this bus without inventing nonexistent equivalents in other clients.

Frontend response and session events remain distinct. Codex and Claude `stop`
events carry the completed turn's `last_assistant_message` and publish as
canonical `message_sent`; their explicit `session_end` publishes as canonical
`session_end`. Grok follows the same distinction: `stop` completes a turn and
does not retire its session route. Claude
`message_display` publishes its rendered `delta` batches as canonical
`message_displayed`. Adapter responses preserve the source `event` separately
from this `canonical_hook`, so authenticated route cleanup follows canonical
lifecycle semantics without erasing frontend provenance. These response events
are observational on this relay: downstream policy may inspect and report
them, but cannot claim to retract text that the frontend has already produced.

Canonical hook subscribers should normally omit `priority`, preserving
independent fan-out. See [`docs/hooks.md`](../../docs/hooks.md) for the complete
composition and priority contract.
