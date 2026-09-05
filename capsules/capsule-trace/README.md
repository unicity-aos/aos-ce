# aos-trace

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
[![MSRV: 1.94](https://img.shields.io/badge/MSRV-1.94-blue)](https://www.rust-lang.org)

**Durable trace and evaluation archive for [Unicity AOS](https://github.com/unicity-aos/aos-ce) agents.**

In the OS model, this capsule is the black-box recorder: a typed capability for turning a session's work into inspectable experience instead of losing it when the session ends. It fills a gap named directly in [`docs/meta-harness.md`](../../docs/meta-harness.md): a meta-harness proposer needs "a unified inventory of harness artifacts, durable trace/evaluation archives" to compare candidates on more than a compressed score.

## Tools

- **`record_trace`** - Append a `trace` or `evaluation` record: summary, outcome, score, cost, tags, candidate id, source ref, and an optional structured `metadata` payload (capped at 8 KB - point large payloads at a file via `source_ref` instead). Returns the assigned `id` and `ts`.
- **`list_traces`** - Filtered, summarized listing (by `kind`, `candidate_id`, `tag`, `since`), newest first, capped at 100 records. Summaries omit `metadata` so a listing call can't flood the context window.
- **`get_trace`** - Fetch one full record, including `metadata`, by `id`.

## Storage

Records are appended as newline-delimited JSON to `cwd://{cwd_dir}/trace.jsonl`, sharing the `cwd_dir` project-folder convention `aos-memory` uses (default `.astrid`). The archive is append-only by design - there is no edit or delete tool. Archived experience is retained evidence for later search and evaluation, not scratch state; an operator who needs to prune the file can do so directly with `aos-fs`.

## Design notes

- **No prompt injection.** Unlike `aos-memory`, this capsule never writes into the system prompt. An agent (or a Forge-built proposer) calls it deliberately when recording or inspecting experience.
- **Bounded reads.** `list_traces` defaults to 20 records and hard-caps at 100, matching the "raw, selectively inspectable experience" principle from the meta-harness research: the proposer decides what to inspect next rather than being handed the whole archive at once.
- **Reject, don't mangle, oversized metadata.** `record_trace` errors if `metadata` exceeds 8 KB serialized rather than silently truncating structured JSON into something invalid.

## Capabilities

`fs_read = ["cwd://"]`, `fs_write = ["cwd://"]` - project-scoped only, no `home://` or `net` access.

## Development

```bash
cargo build --target wasm32-unknown-unknown --release
cargo test
```

## License

Dual-licensed under [MIT](LICENSE-MIT) and [Apache 2.0](LICENSE-APACHE).

Copyright (c) 2026 Marek Sepp and Unicity Labs.
