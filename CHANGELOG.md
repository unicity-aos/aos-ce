# Changelog

## [2026.1.0] - 2026-07-14

### Added

- The `aos` product command and product-owned `~/.unicity-os` state boundary.
- A pinned Unicity CE distribution manifest over Astrid Runtime 0.9.4.
- Reproducible macOS and Linux release bundles with checksums, Sigstore bundles,
  GitHub build-provenance attestations, and explicit runtime/WIT compatibility metadata.
- An idempotent product installer and updater that preserve runtime state while
  replacing the coordinated AOS and Astrid executable set.
