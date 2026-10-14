# AOS Community Edition

AOS gives AI agents a shared local environment for tools, permissions and services.
Connect your agent, approve the access it needs, and give it capabilities you can
inspect and extend. AOS uses the Astrid runtime to execute those capabilities as
isolated WebAssembly capsules with explicit permissions and an audit trail.

Use AOS to:

- **Connect your existing agent to local capabilities.** Oracle plugins connect
  Claude, Codex and Grok to AOS; the product also exposes an MCP interface.
- **Control access to tools and data.** Actions routed through AOS use runtime
  capabilities and approval flows, with a Command Center for local decisions.
- **Keep agent environments separate.** Principals have their own provisioned
  environments, while an authenticated operator manages the installation.
- **Build the missing capability.** Compose capsules into tools, connectors,
  services or agent harnesses. The bundled Forge tooling teaches an agent how
  to build and verify a capsule with only the permissions it needs.

Community Edition includes the `aos` CLI, Command Center, first-party capsules
and a coordinated Astrid runtime. Install it below, then connect your host using
[the Oracle plugins](https://github.com/unicity-aos/oracles#readme).

## Install

The supported installer installs the `aos` product command, its pinned runtime,
and the exact 22 Community Edition capsules built from this source tree under
the product-owned `~/.aos` root:

```sh
curl --proto '=https' --tlsv1.2 -fsSL https://aos.unicity.ai/install.sh | sh
export PATH="$HOME/.aos/bin:$PATH"
aos init
aos status
```

The `export` command makes `aos` available in the current shell. Add that line
to your shell profile to keep it available in new terminals. If you install to
a custom location, use the binary directory printed by the installer.

On macOS, packaged `aos` and bundled `astrid`/`astrid-daemon` binaries declare
`LC_BUILD_VERSION` minos 11.0. Command Center declares `LSMinimumSystemVersion`
13.0 and is not copied onto older hosts. Optional Finder volume mounts follow
`AstridFS.app` `LSMinimumSystemVersion` (currently 26.0). Those floors are
separate: an older Mac can still install the CLI. Native execution on macOS
11–15 is untested. Details: `release/MACOS-SUPPORT.md`.

`aos init`, including `aos init --offline`, provisions from those local,
product-versioned capsule assets. A fresh Community installation automatically
resumes Astrid's ten-capsule batches after each rate-limit window; installing
all 22 capsules includes approximately two minutes of waiting. Completed
installs are not repeated, and a stalled batch remains an error.
Re-running the installer performs a
coordinated product upgrade without
rewriting a standalone runtime installation. Every release publishes
checksums, Sigstore bundles, GitHub build-provenance attestations, and
`runtime-compatibility.toml`, which pins the exact runtime release and WIT commit.
Its machine-readable runtime-compatibility and upgrade/self-heal gates must both
be true before a tag can publish. The latter is approved only after the exact
candidate preserves a frozen standalone-home clone and boots with freshly
generated runtime coordination state.

## Command boundary

AOS owns its product roots, including `init`, `status`, `migrate`, `update`,
`distro`, `mcp`, `daemon`, and `serve-health`:

```sh
aos status
aos status --json
aos --principal codex-code mcp serve
aos daemon foreground --workspace /workspace
```

The Command Center reads selected-principal capsule inventory as its authenticated
local operator, not as the selected agent:
`aos status --json --include-capsules --inventory-principal claude-code`.
The kernel requires authority to inspect another principal. This is read-only,
does not start the runtime, and does not grant the selected principal access to
global status. `--principal` still selects the authenticated status caller.

On Unix, `aos daemon foreground` replaces the AOS process with the persistent
bundled daemon, preserving direct signal and exit-status ownership for process
supervisors. On other hosts it waits for the daemon and preserves its exit
status. Both paths apply the product-owned runtime home, `.aos` workspace
layout, enforced Community Edition distro, and stderr logging environment.
Neither path enables ephemeral lifetime.

Every other runtime root is part of the AOS CLI directly. Arguments, exit
codes, and signals pass through unchanged; there is no nested `aos astrid` or
`aos runtime` namespace:

```sh
aos doctor
aos capsule build
```

When AOS owns a root such as `status`, `init`, `update`, or `mcp`, its product
implementation replaces the lower-level command at that same location. The
complete supported surface therefore remains `aos <verb>`. Release validation
compares the exact pinned runtime's public command inventory with AOS's
classified root contract, so a new runtime verb cannot enter a product release
without an explicit inherit-or-own decision.

`aos mcp serve` is the product edge shared by Codex, Claude, and Grok. A client
that supports MCP form elicitation keeps presenting its own constrained
approval forms. When a client does not, the default `--interaction auto` mode
uses a local AOS decision surface: AppKit on macOS, a native Windows dialog, or
Pinentry on Linux. `--interaction client`, `native`, and `deny` make the policy
explicit. The local bridge accepts only a single boolean or the fixed AOS
approval enum; arbitrary strings, password-shaped fields, and URL
elicitations are never collected through it.

## Build on AOS

Unicity AOS is the operating system in which agents and agent-native software
run. Capsules are general user-space building blocks: users can compose them
into harnesses, meta-harnesses, connectors, services, or other systems.
Community Edition ships Forge as OS construction tooling so a fresh agent can
inspect the running system, learn the capsule model, identify a real capability
gap, and build and verify a least-privilege capsule. Forge also installs the
`meta-harness` skill, which teaches an agent how to build a governed
meta-harness on AOS by treating its instructions, memory, skills, harness code,
tools, capsules, traces, and evaluations as an improvable user-space world. The
agent is instructed to notice useful extensions during real work and reach for
Forge proactively when new code is the right way to improve that world.

See [Extending an agent's world on AOS](docs/meta-harness.md) for the world
model, research loop, Forge boundary, optional worker pattern, and
representative user experiences.

Provisioning another principal keeps the authenticated operator separate from
the target environment:

```sh
aos --principal operator init --target-principal alice
```

This AOS release fixes its distribution state to Unicity CE. Use a standalone
`astrid` installation and runtime home to apply another distribution. Homebrew
installations update with `aos update`. Direct installs resolve the signed
channel matching their installed build by default (`stable`, `dev` for RCs,
or `nightly`) and can explicitly select another channel or an exact version;
all remain fail-closed until their signed metadata is actually published. See
[Signed AOS release channels](docs/release-channels.md).

Both `aos version` and `aos --version` print the AOS product version and a
labelled bundled Astrid runtime version without starting the runtime or checking
for updates. `aos version --format json` reports `version` (AOS) and
`runtime_version` (Astrid).

For direct installations, `aos update --check` authenticates the signed channel
matching the installed build and reports availability without installing, restarting the runtime,
or changing accepted channel state. Use `--channel dev` or `--channel nightly`
to check another channel. A failed check exits nonzero; it is not evidence that
the installation is up to date. The check requires the installed product updater.

`aos update --check --json` exposes the same authenticated check to integrations.
On success stdout contains one JSON object with `schema_version: 1`, the installed
and channel versions, channel, target, and channel-selected artifact SHA-256.
`verification: "metadata"` means the signed metadata passed verification, **not**
that the product archive was downloaded or installed. Diagnostics go to stderr;
an unsuccessful check has a nonzero exit and must not be interpreted as current.
The two versions are deliberately separate: a differing channel version may be
older than the installed version. JSON output requires `--check` and never applies
an update.

## Import an existing runtime

The `aos` CLI can deliberately copy compatible state from a standalone runtime
installation without changing the source. See
[Importing standalone runtime state](docs/runtime-migration.md) for the exact
allowlist, integrity checks, recovery behavior, and command.

## License

Except where otherwise noted, AOS Community Edition is dual-licensed under the
[MIT License](LICENSE-MIT) or the [Apache License 2.0](LICENSE-APACHE), at your
option.

## Repository layout

```text
crates/       Product CLI, HTTP API, control client, and shared product code
capsules/     First-party production capsules
distros/      Community distribution manifests and release metadata
docs/         Product and operator documentation
```
