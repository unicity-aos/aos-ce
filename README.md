# astrid-capsule-openai-compat

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

**The OpenAI-compatible LLM provider for [Astrid OS](https://github.com/unicity-astrid/astrid).**

In the OS model, this capsule is a device driver. It translates between Astrid's standardized LLM event protocol and any OpenAI-compatible Chat Completions API — the same way a device driver translates between an OS and hardware.

Configure `base_url` to point at any compatible provider:

| Provider | `base_url` |
|---|---|
| OpenAI | `https://api.openai.com` |
| Groq | `https://api.groq.com/openai` |
| Together | `https://api.together.ai` |
| Mistral | `https://api.mistral.ai` |
| DeepSeek | `https://api.deepseek.com` |
| Fireworks | `https://api.fireworks.ai/inference` |

Set `base_url` to the provider **origin only** — the capsule appends `/v1/chat/completions` itself, so do not include a `/v1` suffix.

## How it works

1. Subscribes to `llm.v1.request.generate.openai-compat` IPC events
2. Converts Astrid's `Message` format to the OpenAI Chat Completions JSON format (text, tool calls, tool results, multipart)
3. Opens a streaming HTTP connection to `{base_url}/v1/chat/completions` via the HTTP streaming airlock
4. Parses the SSE response in real-time and publishes standardized `llm.v1.stream.openai-compat` events back to the IPC bus as chunks arrive

Stream events cover the full response lifecycle: text deltas, parallel tool call start/delta/end, usage reporting (prompt + completion tokens), and completion.

## Configuration

The capsule prompts for these environment variables during `astrid init`; every field except `api_key` has a default.

| Variable | Type | Default | Description |
|---|---|---|---|
| `api_key` | secret | — | Provider API key, sent as `Authorization: Bearer …` |
| `base_url` | string | `https://api.openai.com` | Provider origin **without** `/v1` — the capsule appends `/v1/chat/completions` |
| `model` | string | `gpt-5.4` | Default model ID; a request may override it per call |
| `context_window` | integer | `128000` | Context window (tokens) advertised to the provider registry |
| `max_output_tokens` | integer | `8192` | Sent as `max_tokens` on each request |
| `temperature` | string | _(unset)_ | Default sampling temperature (`0.0`–`2.0`); blank uses the provider default |

`model` is required on every Chat Completions request (`CreateChatCompletionRequest.required = [model, messages]`). The capsule resolves it as request value → `model` env default → `gpt-5.4`.

## IPC protocol

| Direction | Topic | Payload |
|---|---|---|
| Subscribe | `llm.v1.request.generate.openai-compat` | `IpcPayload::LlmRequest` |
| Publish | `llm.v1.stream.openai-compat` | `IpcPayload::LlmStreamEvent` |

## Development

```bash
rustup target add wasm32-unknown-unknown
cargo build --target wasm32-unknown-unknown --release
```

## License

Dual-licensed under [MIT](LICENSE-MIT) and [Apache 2.0](LICENSE-APACHE).

Copyright (c) 2025-2026 Joshua J. Bouw and Unicity Labs.
