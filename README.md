# astrid-capsule-registry

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

**The LLM provider registry for [Astrid OS](https://github.com/unicity-astrid/astrid).**

In the OS model, this capsule is the device manager. It discovers which LLM provider capsules are loaded, resolves their IPC routing topics, and manages which model is currently active.

## How it works

1. Waits for `astrid.v1.capsules_loaded` from the kernel (all capsules booted)
2. Queries the kernel for capsule metadata via `GetCapsuleMetadata`
3. Resolves provider entries: model ID, description, capsule name, request/stream topics, capabilities
4. Persists the provider list and active model in the capsule KV store
5. Auto-selects the sole provider when only one is available

On capsule reload events, the registry re-discovers providers, clears stale active model references, and auto-selects again if applicable.

## IPC protocol

| Direction | Topic | Description |
|---|---|---|
| Subscribe | `registry.v1.get_providers` | Returns the provider list |
| Subscribe | `registry.v1.get_active_model` | Returns the active provider |
| Subscribe | `registry.v1.set_active_model` | Sets active model by ID |
| Publish | `registry.v1.active_model_changed` | Emitted on model switch |
| Publish | `registry.v1.response.*` | Per-request responses |

## CLI integration

Handles the `/models` command:
- `/models` - emits a `SelectionRequired` payload for the TUI picker
- `/models <model_id>` - direct model switch

## Security

Only accepts capsule metadata responses from the kernel's system session UUID. Messages from untrusted sources are logged and discarded.

## Development

```bash
rustup target add wasm32-unknown-unknown
cargo build --target wasm32-unknown-unknown --release
```

## License

Dual-licensed under [MIT](LICENSE-MIT) and [Apache 2.0](LICENSE-APACHE).

Copyright (c) 2025-2026 Joshua J. Bouw and Unicity Labs.
