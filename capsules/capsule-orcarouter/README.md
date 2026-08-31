# aos-orcarouter

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

**The OrcaRouter LLM provider for [Unicity AOS](https://github.com/unicity-aos/aos-ce).**

In the OS model, this capsule is a device driver. It translates between the runtime's standardized LLM
event protocol and the OrcaRouter gateway's OpenAI-compatible Chat Completions API — the same way a
device driver translates between an OS and hardware.

OrcaRouter is an OpenAI-compatible AI gateway built for both models and agents. Like OpenRouter, it
exposes a provider/model namespace across many models — but it also combines adaptive routing,
automatic failover, zero-markup inference, observability, guardrails, and agent-tool governance
behind the same endpoint. Adding `orcarouter` as a first-class provider means this project's users
can use that stack directly, without treating OrcaRouter as an anonymous custom base URL.

The gateway origin is `https://api.orcarouter.ai` — the capsule appends `/v1/chat/completions` for
generation and `/v1/models` for discovery.

## How it works

1. Subscribes to `llm.v1.request.generate.orcarouter` IPC events
2. Converts the runtime's `Message` format to the OpenAI Chat Completions JSON format (text, tool calls,
   tool results, multipart)
3. Opens a streaming HTTP connection to `https://api.orcarouter.ai/v1/chat/completions` via the HTTP
   streaming airlock
4. Parses the SSE response in real-time and publishes standardized `llm.v1.stream.orcarouter`
   events back to the IPC bus as chunks arrive

Stream events cover the full response lifecycle: text deltas, parallel tool call
start/delta/end, usage reporting (prompt + completion tokens), and completion.

## Model discovery

When the registry asks this capsule what it can serve, the capsule queries
`GET https://api.orcarouter.ai/v1/models` and returns one provider entry per discovered model id
(e.g. `orcarouter/free`, `orcarouter/fusion`). Every entry shares the same request and stream topics;
the entry id IS the model id.

Discovery runs at describe-time (when the registry fans out `llm.v1.request.describe`), not at
startup. The env `model` value controls two things during discovery:

- **Ordering.** If the env model appears in the discovered list, its entry is emitted first
  (`entry[0]`), so the registry can pre-select it. All other models keep their upstream order.
- **Offline fallback.** If `https://api.orcarouter.ai/v1/models` is unreachable or returns an error,
  the capsule falls back to advertising a single entry for the configured env model. This keeps
  existing pinned installs working when the gateway is temporarily down.

If discovery fails AND no env model is configured, the capsule advertises nothing rather than
invent a bogus id the registry could send upstream verbatim.

## Configuration

These fields are prompted during `aos init`. Every field except `api_key` has a default or can be
left blank.

| Variable | Type | Default | Description |
|---|---|---|---|
| `api_key` | secret | -- | OrcaRouter API key, sent as `Authorization: Bearer ...` |
| `model` | select | `orcarouter/free` | Default model; populated live from `https://api.orcarouter.ai/v1/models` during onboarding |
| `context_window` | integer | `128000` | Context window (tokens) advertised to the registry |
| `max_output_tokens` | integer | `8192` | Sent as `max_tokens` on each request |
| `temperature` | string | _(unset)_ | Sampling temperature (`0.0`--`2.0`); blank uses the provider default |

### The `model` field is a live select

During `aos init`, the installer fetches `https://api.orcarouter.ai/v1/models` (using the entered
`api_key`) and presents a numbered menu of available models. The configured `model` default is
pre-selected. If the endpoint cannot be reached the installer falls back to free-text entry.

## Selecting a model at runtime

Model selection is per-principal and stored in the registry capsule's KV store. OrcaRouter models
are qualified with the provider alias (`orcarouter:<model>`) only when a bare id would be ambiguous
across providers:

```sh
# Select an OrcaRouter model by bare id (when unambiguous)
aos models set orcarouter/free

# Disambiguate when another provider serves the same model name
aos models set orcarouter:orcarouter/free
```

## IPC protocol

| Direction | Topic | Payload |
|---|---|---|
| Subscribe | `llm.v1.request.generate.orcarouter` | `IpcPayload::LlmRequest` |
| Subscribe | `llm.v1.request.describe` | describe request (registry fan-out) |
| Publish | `llm.v1.stream.orcarouter` | `IpcPayload::LlmStreamEvent` |
| Publish | `llm.v1.response.describe` | provider descriptor array |

## Development

```bash
rustup target add wasm32-unknown-unknown
cargo build
```

## License

Dual-licensed under [MIT](LICENSE-MIT) and [Apache 2.0](LICENSE-APACHE).

Copyright (c) 2025-2026 Joshua J. Bouw and Unicity Labs.
