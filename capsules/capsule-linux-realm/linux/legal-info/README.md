# Linux-bearing artifact legal material

This directory retains the compact, reviewable output for the exact embedded
artifacts:

- `buildroot.config`, `manifest.csv`, and `licenses/` came from Buildroot
  2026.05.1 `make legal-info` after producing the recorded rootfs;
- `BUILDROOT-README` is Buildroot's generated explanation and warning ledger;
  and
- `linux-6.18.39/` retains the kernel's license pointer and GPL-2.0 text.

This subset is evidence, not by itself a binary-distribution compliance bundle.
The generated target and host source trees are about 454 MiB and are not checked
into this repository. Before distributing the capsule, release packaging must
regenerate `make legal-info`, add the exact Buildroot 2026.05.1 source tree, add
the exact Linux 6.18.39 source tree, review Buildroot's warnings, and publish the
corresponding source bundle alongside the binary artifact. `SOURCES.lock` and
`DEVELOPMENT.lock` provide the upstream and output identities needed for that
release gate.
