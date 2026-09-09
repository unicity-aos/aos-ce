# Changelog

Notable changes to AOS are recorded here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versions follow [year.month.patch](release/VERSIONING.md). Repairs to unreleased
implementations are consolidated into their final behavior.

[Unreleased]: https://github.com/unicity-aos/aos-ce/compare/2026.9.0...HEAD
[2026.9.0]: https://github.com/unicity-aos/aos-ce/compare/2026.1.3...2026.9.0
[2026.1.3]: https://github.com/unicity-aos/aos-ce/releases/tag/2026.1.3

## [Unreleased]

### Fixed

- Complete Community initialization in one AOS invocation by resuming the
  runtime's bounded installation batches without reinstalling completed capsules.

## [2026.9.0] - 2026-09-09

### Added

- Signed macOS filesystem app installation through the common installer,
  including website and Oracle bootstrap paths. AOS.app retains Astrid's signed
  internal identity; macOS approval failures include retry instructions.
- Native Linux musl product archives for x86_64 and ARM64, authenticated platform
  selection, and bundled FUSE providers. GNU Linux and Darwin bundles include
  their corresponding filesystem providers.
- `aos distro apply --principal P --yes` for applying the signed Community
  inventory with the selected principal and an AOS activation receipt.
- The AOS-owned MCP broker and host interaction bridge for Codex, Claude, and
  Grok. Hosts retain separate principal capsule sets rather than receiving
  every installed Community capsule.
- Authenticated host hook ingress, protocol-specific adapters, and session-bound
  meta-harness context delivery.
- `aos daemon foreground` for supervisors, with signal and exit-status ownership.
- Bounded Rhai evaluation and semantic surface/catalog components. Presence in
  source does not imply inclusion in the default Community distribution.

### Changed

- Pin the runtime and Rust client dependencies to published Astrid 2026.9.0,
  including authenticated GNU, Darwin, and musl release metadata.
- Runtime executables live in immutable versioned release directories, separate
  from durable runtime state. A stopped runtime contains only its private,
  non-empty `astrid.volume`.
- Community publication selects 22 capsule artifacts. Oracle packs independently
  require `aos-mcp` and `aos-skills`, with `aos-forge` when present.
- Signed release metadata binds packaged executable bytes and capsule inventory.
  Installations preserve authenticated Distro manifests, locks, and signatures.
- Agent skills remain distinct from capsule authority and installation.
- Community Edition's repository license is explicitly MIT OR Apache-2.0.
- The AOS product advances from 2026.1.3 to 2026.9.0. The
  [dependency version inventory](release/DEPENDENCIES-2026.9.0.md) records source
  lockfile changes; production Astrid pins remain a release-time dependency.
- Hosts without MCP form support use native approval fallbacks: AppKit on
  macOS, native confirmation on Windows, or Pinentry on Linux. The interaction
  bridge refuses free-form and secret-shaped fields.
- Runtime compatibility targets Astrid 2026.9.0 and Oracle 2026.9.0. See the
  [release requirements](release/RELEASE-2026.9.0.md) for platform setup and
  publication dependencies; Windows installation is not certified by this cut.

### Removed

- The vendored Telegram capsule; it is maintained in its separate repository.

### Fixed

- MCP forwarding preserves framing, workspace and request-timeout arguments,
  stderr, and child exit/signal results.
- Every delegated stop is confirmed before reporting successful shutdown.
- Status uses the selected principal rather than silently assuming default.
- GNU builds support enterprise Linux with glibc 2.34; musl targets use native
  musl binaries rather than relabeled GNU archives.
- Required filesystem provider executables survive installation and self-heal.

## [2026.1.3] - Unreleased

### Added

- The `aos` product command and product-owned `~/.aos` state boundary.
- A pinned Unicity CE distribution manifest over Astrid Runtime 0.10.4, emplaced
  as the bundled runtime's operator-enforced distro.
- Reproducible macOS and Linux release bundles with primary BLAKE3 and
  Homebrew-compatible SHA-256 checksum manifests, Sigstore bundles, GitHub
  build-provenance attestations, and explicit runtime/WIT compatibility
  metadata.
- An idempotent product installer and updater that preserve runtime state while
  replacing the coordinated AOS and Astrid executable set and atomically
  installing the product-versioned Community Edition capsule set.
- Schema-3 runtime-import receipts with canonical `blake3:<hex>` content digests
  and fail-closed rejection of pre-release SHA-256 receipts.
- Runtime import holds the standalone daemon's existing singleton lock without
  changing the source, and interrupted unreceipted cutovers always roll back
  before recopying the current locked source.
- A signed release path for the 19 installable `aos-*` artifacts
  built from this source tree and selected locally by Community Edition, with
  exact source/manifest identity checks, product-archive inclusion, offline
  provisioning, archive safety validation, BLAKE3 checksums, SHA-256
  compatibility checksums, Sigstore bundles, and provenance.
- Host-target unit-test coverage for the capsule workspace.
- Forge in the default Community Edition distribution, including a discoverable
  bootstrap and skill that teach fresh agents to build a user-space
  meta-harness on AOS by seeing instructions, memory, skills, harness code,
  tools, capsules, traces, and evaluations as an improvable world. Agents reach
  for Forge proactively when real work reveals a useful new capability, while
  optional workers remain a use-case choice rather than a prerequisite.
- Homebrew formula updates initiated by the tap's authenticated stable-release
  poll, eliminating the cross-repository dispatch credential.
- Strict, signed stable/dev/nightly channel and immutable release metadata
  contracts with exact workflow identities, expiry, replay-resistant generation
  state, and fail-closed direct installer resolution.
- A native release gate that initializes a clean AOS home, verifies the exact
  19-capsule CE lock, grants, and ready set, repeats initialization without
  changing runtime state, and proves clean daemon shutdown before publication.
- Native `aos status` output for authenticated running state and verified
  stopped state without invoking the runtime CLI.
- An opt-in daily nightly train with deterministic run-dated versions, exact
  Astrid compatibility pins, protected publication and promotion, and
  idempotent recovery after interrupted release or pointer updates. It is
  disabled by default; merging `main` never publishes a release.

### Changed

- Keep agent Skills out of `Capsule.toml` and the generic capsule release
  contract. Host plugins may vendor trigger Skills, the AOS Skills service
  indexes workspace and principal-home entries, and capsules expose detailed
  guidance over ordinary IPC tools without teaching the runtime an AI-specific
  file protocol.
- Make Capsule Forge a progressively disclosed, exhaustive AOS author manual:
  its compact Skill now routes fresh agents into installed reference chapters
  covering portable source placement, all manifest capabilities, IPC
  layering/priority, WIT, Skills and host plugins, construction-versus-activation
  authority, build/release practice, security, and proactive meta-harness design.
- Target the Telegram capsule's actual `aos-telegram` package name in CI so the
  WASI job and target-specific workspace exclusions run as intended.
- Parse AOS-owned commands with Clap-generated validation and help while
  preserving byte-for-byte delegation of inherited runtime commands and their
  help surfaces.
- Initialize against the operator-enforced Community Edition manifest and ask
  the runtime to grant its installed capsule set to the resolved target
  principal.
- Bootstrap the default CE system fleet through Astrid's canonical distro
  installer before the daemon-backed authorization pass, allowing a completely
  fresh AOS home and non-default targets to use the same runtime trust path.
- Keep the authenticated init operator separate from its target principal,
  prevent AOS distribution replacement, and fail closed while signed direct
  update channels remain unpublished.
- Require explicit machine-readable runtime-compatibility and upgrade/self-heal
  approvals before the tag-triggered workflow can package or publish a release,
  backed by a packaged migration/reinstall test over the frozen 2026-07-15
  Astrid 0.9.4 home shape and a final-candidate runtime boot hook.
- Present product-facing capsule copy consistently as Unicity AOS while
  preserving stable Astrid Runtime crate, WIT, topic, artifact, and ABI names.
- Treat the runtime's expected shutdown-response disconnect as a successful
  `aos stop` only after every coordination marker is gone and the singleton
  lock is available; all other inherited runtime failures retain their output
  and exit status.
