# Changelog

## [2026.1.0] - Unreleased

### Added

- The `aos` product command and product-owned `~/.unicity-os` state boundary.
- A pinned Unicity CE distribution manifest over Astrid Runtime 0.9.4.
- Reproducible macOS and Linux release bundles with primary BLAKE3 and
  Homebrew-compatible SHA-256 checksum manifests, Sigstore bundles, GitHub
  build-provenance attestations, and explicit runtime/WIT compatibility
  metadata.
- An idempotent product installer and updater that preserve runtime state while
  replacing the coordinated AOS and Astrid executable set.
- Schema-3 runtime-import receipts with canonical `blake3:<hex>` content digests
  and fail-closed rejection of pre-release SHA-256 receipts.
- A signed release path for the 18 installable `astrid-capsule-*` artifacts
  selected by Community Edition, with exact source/manifest identity checks,
  archive safety validation, BLAKE3 checksums, SHA-256 compatibility checksums,
  Sigstore bundles, and provenance.
- Host-target unit-test coverage for the capsule workspace.
