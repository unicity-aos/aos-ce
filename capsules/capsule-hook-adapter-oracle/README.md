# aos-hook-adapter-oracle

Translates authenticated Codex, Claude, and Grok frontend hook envelopes into
the canonical AOS `hook.v1.event.*` protocol.

The adapter owns protocol translation only. It does not own downstream hook
policy and it does not turn observation events into authorization decisions.
`user_prompt_submit` collects bounded `additional_context` replies for the exact
host turn. `pre_tool_use` and `permission_request` are observation-only on this
relay; binding native-tool denial remains on the `astrid-gate`/broker decision
path, whose response schema can express deny.

Frontend response and session events remain distinct. Codex and Claude `stop`
events carry the completed turn's `last_assistant_message` and publish as
canonical `message_sent`; their explicit `session_end` publishes as canonical
`session_end`. Grok is an explicit compatibility exception: its current `stop`
event maps to canonical `session_end` because the installed Grok hook contract
does not expose a distinct verified termination event. Claude
`message_display` publishes its rendered `delta` batches as canonical
`message_displayed`. Adapter responses preserve the source `event` separately
from this `canonical_hook`, so authenticated route cleanup follows canonical
lifecycle semantics without erasing frontend provenance. These response events
are observational on this relay: downstream policy may inspect and report
them, but cannot claim to retract text that the frontend has already produced.

Canonical hook subscribers should normally omit `priority`, preserving
independent fan-out. See [`docs/hooks.md`](../../docs/hooks.md) for the complete
composition and priority contract.
