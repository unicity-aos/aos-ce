# aos-hook-bridge

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

The canonical Astrid lifecycle-to-hook router for Unicity AOS.

Frontend protocols are translated by adapter capsules such as
`aos-hook-adapter-oracle`. This capsule maps only kernel lifecycle events onto
`hook.v1.event.*`, collects correlation-scoped responses for binding lifecycle
hooks, and applies their merge semantics.

Observation hooks are priority-free fan-out. Binding lifecycle hooks use an
explicit request/response protocol: `before_tool_call` is deny-wins and
last-transform-wins; result/content transformations use last-non-null.

See [`docs/hooks.md`](../../docs/hooks.md) for the architecture, response
semantics, extension model, and priority rules.

## License

Dual-licensed under [MIT](LICENSE-MIT) and [Apache 2.0](LICENSE-APACHE).

Copyright (c) 2025-2026 Joshua J. Bouw and Unicity Labs.
