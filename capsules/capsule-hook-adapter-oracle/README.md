# aos-hook-adapter-oracle

Translates authenticated Codex, Claude, and Grok frontend hook envelopes into
the canonical AOS `hook.v1.event.*` protocol.

The adapter owns protocol translation only. It does not own downstream hook
policy and it does not turn observation events into authorization decisions.
`user_prompt_submit` collects bounded `additional_context` replies for the exact
host turn. `pre_tool_use` and `permission_request` are observation-only on this
relay; binding native-tool denial remains on the `astrid-gate`/broker decision
path, whose response schema can express deny.

Canonical hook subscribers should normally omit `priority`, preserving
independent fan-out. See [`docs/hooks.md`](../../docs/hooks.md) for the complete
composition and priority contract.
