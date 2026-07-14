# AOS Community Edition

AOS Community Edition is the open agent operating system for people who want
an inspectable, composable environment for agents.

It owns the Community Edition product surface: the `aos` CLI, HTTP API,
distributions, first-party capsules, provider and model experience, and
Unicity Audit.

## Workspace layout

```text
crates/       Product CLI, HTTP API, control client, and shared product code
capsules/     First-party production capsules
distros/      Community distribution manifests and release metadata
docs/         Product and operator documentation
```

## Import an existing runtime

The `aos` CLI can deliberately copy compatible state from a standalone Astrid
Runtime installation without changing the source. See
[Importing standalone runtime state](docs/runtime-migration.md) for the exact
allowlist, integrity checks, recovery behavior, and command.
