# aos-system

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

**System management tools for [Unicity AOS](https://github.com/unicity-aos/aos-ce) agents.**

This capsule gives agents typed tools to inspect their principal's last observed loaded capsule view, read interface contracts, and check that view's interface coverage. It requires Astrid 0.9.0 or newer.

## Tools

| Tool | Description |
|---|---|
| `list_capsules` | List names and versions from the principal's last observed loaded snapshot |
| `inspect_capsule` | Return runtime-supplied installed metadata, principal, and observation time; not Capsule.toml |
| `list_interfaces` | List available WIT interface definitions |
| `read_interface` | Read a WIT interface definition (typed contract between capsules) |
| `system_status` | Loaded-snapshot capsule count, interface coverage, unsatisfied imports, principal, and observation time |

## How it works

Capsule inspection consumes Astrid's principal-stamped `astrid.v1.capsules_loaded` events and caches metadata in principal-and-capsule-scoped KV. Shared executable hashes remain references: no capsule binaries or private capsule directories are copied into a principal's home. Live tool descriptors are omitted from this inspection cache.

The snapshot describes loaded capsules, not every installed artifact in the global store. An empty snapshot is a valid empty loaded view; an absent snapshot or unavailable installed metadata returns an error. A tools-only discovery result does not count as installed metadata. Observation time is not a live health probe or proof that a later load/unload has already been observed.

WIT inspection still reads `home://wit/astrid/` through the kernel's VFS and capability system.

The LLM uses these tools to understand its own runtime before making changes. A typical flow:

1. `list_capsules` -- see the last observed loaded view
2. `read_interface session` -- understand the session interface contract
3. `inspect_capsule aos-session` -- inspect its metadata and shared executable hash
4. Build and install a replacement (via shell or future system tools)

## Security

- Read-only VFS access (`fs_read = ["home://"]`)
- Snapshot KV storage is scoped to the invocation principal and this capsule
- Path traversal rejected on all inputs
- No capability to modify capsules directly -- write operations require separate tools with approval gates

## Development

```bash
cargo build --target wasm32-unknown-unknown --release
```

## License

Dual-licensed under [MIT](LICENSE-MIT) and [Apache 2.0](LICENSE-APACHE).

Copyright (c) 2025-2026 Joshua J. Bouw and Unicity Labs.
