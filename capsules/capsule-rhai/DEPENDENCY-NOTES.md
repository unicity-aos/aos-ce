# Dependency notes

## Rhai and `smartstring`

The capsule is pinned to Rhai `1.26.0`.  That release has a mandatory
dependency on `smartstring 1.0.1`, which is reported as unmaintained by
RustSec advisory `RUSTSEC-2026-0249` (advisory date 2026-05-03).  The current
crates.io release is still `1.0.1`, and Rhai 1.26 does not expose a feature to
replace its string representation.  Replacing it locally would require a
vendored Rhai/smartstring fork and would expand the reviewed code surface.

This is therefore an accepted maintenance risk, not a claim that the advisory
is fixed.  `cargo audit` must continue to report the warning, and release
selection must keep `aos-rhai` out of any allowlist until a maintained Rhai
dependency path is available.  Re-check the RustSec advisory and Rhai's
dependency graph whenever Rhai or this capsule is upgraded; close this note
when the upstream dependency is replaced or removed.

The unrelated `chacha20 0.10.1` yanked-package warning comes from the existing
Astrid runtime dependency graph and is outside this capsule's change scope.
