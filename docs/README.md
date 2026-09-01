# AOS Community Edition documentation

Product and operator documentation for Unicity AOS Community Edition.

Published product identity on `main` is `2026.1.3`, pinned to Astrid Runtime
`0.10.4`. Later identities such as AOS `2026.9.0` are accepted release
targets, not published tags.

| Page | What it covers |
|---|---|
| [Signed release channels](release-channels.md) | Current `stable` / `dev` / `nightly` pointers and the published `YYYY.MINOR.PATCH` rule |
| [Runtime layout](runtime-layout.md) | Current `~/.aos` layout versus the accepted single-volume layout, which is not shipped |
| [Linux Realm consumer contract](principal-linux-realm.md) | Accepted Linux Realm boundary; not implemented on current `main` |
| [Importing standalone runtime state](runtime-migration.md) | Copying compatible state from `~/.astrid` into `~/.aos` |
| [Extending an agent's world](meta-harness.md) | Forge / meta-harness world model |
| [Hooks](hooks.md) | Host hook routing |

## Host MCP paths

Claude Code and Grok Build use `aos --principal <host>-code mcp serve`.
Codex's default Oracle path is a persistent `mcp attach` through the host
plugin (`aos-up` and its stdio frame). Explicit `mcp serve` remains the Codex
escape hatch. Installing or updating the Codex plugin does not inject tools
into an already-running session; start a fresh Codex thread after enablement.

The 2026.9.0 manual signed-install journey (verify artifacts, install with the
daemon stopped, enable the plugin, then start a fresh Codex session) is not
shipped.
