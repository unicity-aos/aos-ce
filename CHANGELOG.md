# Changelog

## [2026.1.0] - Unreleased

### Added

- The `aos` product command and product-owned `~/.aos` state boundary.
- A pinned Unicity CE distribution manifest over Astrid Runtime 0.9.4, emplaced
  as the bundled runtime's operator-enforced distro.
- Reproducible macOS and Linux release bundles with primary BLAKE3 and
  Homebrew-compatible SHA-256 checksum manifests, Sigstore bundles, GitHub
  build-provenance attestations, and explicit runtime/WIT compatibility
  metadata.
- An idempotent product installer and updater that preserve runtime state while
  replacing the coordinated AOS and Astrid executable set and atomically
  installing the product-versioned Community Edition capsule set.
- A signed release path for the 18 installable `aos-*` artifacts
  built from this source tree and selected locally by Community Edition, with
  exact source/manifest identity checks, product-archive inclusion, offline
  provisioning, archive safety validation, BLAKE3 checksums, SHA-256
  compatibility checksums, Sigstore bundles, and provenance.
- Host-target unit-test coverage for the capsule workspace.
- Homebrew formula updates initiated by the tap's authenticated stable-release
  poll, eliminating the cross-repository dispatch credential.
- Strict, signed stable/dev/nightly channel and immutable release metadata
  contracts with exact workflow identities, expiry, replay-resistant generation
  state, and fail-closed direct installer resolution.

### Changed

- **Fresh installs provision the signed local control capsule before first
  init.** This lets the bundled runtime start its authenticated control uplink
  for grant preflight without importing a standalone Astrid home or relying on
  a legacy capsule package name.

- Parse AOS-owned commands with Clap-generated validation and help while
  preserving byte-for-byte delegation of inherited runtime commands and their
  help surfaces.
- Initialize against the operator-enforced Community Edition manifest and ask
  the runtime to grant its installed capsule set to the resolved target
  principal.
- Keep the authenticated init operator separate from its target principal,
  prevent AOS distribution replacement, and fail closed while signed direct
  update channels remain unpublished.
- Require explicit machine-readable runtime-compatibility and upgrade/self-heal
  approvals before the tag-triggered workflow can package or publish a release,
  backed by a packaged clean-install/reinstall test that proves standalone
  Astrid state remains untouched and a final-candidate runtime boot hook.
- Present product-facing capsule copy consistently as Unicity AOS while
  preserving stable Astrid Runtime crate, WIT, topic, artifact, and ABI names.
