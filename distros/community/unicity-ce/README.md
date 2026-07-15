# Unicity CE

The flagship Unicity CE distribution — a curated bundle of
capsules for the complete agent operating system experience.

## What is this?

Unicity CE is a **distro manifest** — a `Distro.toml` file that declares which
capsules to install, their versions, and how they connect. It is not code. It is
product metadata that `aos init` reads to set up a working environment.

## Quick start

```bash
aos init
```

`aos init` uses the manifest and capsule assets embedded in the installed AOS
release, prompts you to select providers (for example, which LLM backend), and
installs everything without following a mutable repository source. The same
local bundle supports `aos init --offline`.

## What's included

| Category | Capsules |
|----------|----------|
| **Uplinks** | cli, registry |
| **LLM providers** | openai-compat (select during init) |
| **Core** | react, session, identity, users, router, prompt-builder, context-engine, hook-bridge |
| **Tools** | shell, http, fs, system |
| **Extensions** | skills, agents, memory |

## Customising

Changes to `Distro.toml` define the next source-built Community Edition bundle.
The release contract requires a one-to-one mapping between every selected
capsule and the installable `.capsule` artifact included in each product
archive. A live AOS installation does not accept distribution replacement.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE), at your option.
