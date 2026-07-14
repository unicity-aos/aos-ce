# Changelog

## [2026.1.0] - Unreleased

### Added

- The `aos` product command and product-owned `~/.unicity-os` state boundary.
- A pinned Unicity CE distribution manifest over Astrid Runtime 0.9.4.
- Reproducible macOS and Linux release bundles with checksums, Sigstore bundles,
  GitHub build-provenance attestations, and explicit runtime/WIT compatibility metadata.
- An idempotent product installer and updater that preserve runtime state while
  replacing the coordinated AOS and Astrid executable set.
- A signed release path for the 18 installable `astrid-capsule-*` artifacts
  selected by Community Edition, with exact source/manifest identity checks,
  archive safety validation, checksums, Sigstore bundles, and provenance.
- Host-target unit-test coverage for the capsule workspace.
