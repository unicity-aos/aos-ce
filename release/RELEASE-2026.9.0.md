# AOS 2026.9.0 release preparation

This source PR is not publication authorization. Publish in dependency order:
Astrid 2026.9.0, AOS 2026.9.0, then Oracle 2026.9.0.

## Before enabling publication

- Rehearse the selected candidate on macOS and GNU/musl x86_64 and ARM64:
  installation, old-layout migration, start/stop, and filesystem write/sync/
  unmount. Record source identities and rehearsal digests separately from
  future production bytes.
- Exercise Codex, Claude, and Grok through their real Oracle adapters. The
  host-specific capsule grants are not the whole Community distribution.
- Confirm first-install batching and installation from a project directory;
  retries or runtime-directory workarounds are not one-shot success.
- After Astrid publication, authenticate both its legacy and musl metadata.
  Update the exact runtime compatibility pins, Cargo dependencies and lock,
  Distro Astrid requirement, runtime command inventory, and bootstrap runtime
  constant together to 2026.9.0. Do not invent missing production hashes or
  replace production trust with the disposable rehearsal key.
- Run `python3 scripts/validate-release-contract.py --require-release-ready`
  and the package/installer contracts with those actual inputs. Set readiness
  flags only when the corresponding evidence exists.
- Confirm all 22 selected capsule artifacts and filesystem providers are in
  the authenticated archives. Verify production signatures during authorized
  release execution; a failure blocks publication/downstream consumption.

## Changelog policy for this cut

`CHANGELOG.md` describes net changes since tag `2026.1.3`; its historical
section is restored from that tag. Development-only signing rehearsals,
temporary source repins, fixture repairs, and fixes to newly introduced
features are folded into the final behavior, not advertised as separate fixes
to an older public release. Source history retains those details.

## Published upstream identity

Astrid v2026.9.0 is an immutable published release at
`7bad449122c08373e8fe4024a80f97a8787c2a44`. Both metadata bundles were verified
against the release workflow's Sigstore identity before pinning:

- GNU/Darwin metadata BLAKE3: `66f7c7b9fabb17cc521e542fc524b9e9deca42826809306f6fd0096685d9e62e`.
- Musl metadata BLAKE3: `92fba87b0c94ff4bc41e9558e65295c2b37ef0174f875b6460be984fd9963de2`.

The four direct Rust client dependencies and Cargo.lock now use published
2026.9.0 crates. Distro, bootstrap constant and command inventory agree; the
inventory includes inherited `storage`. Production publication still executes
the signing and archive checks. No Windows product certification is implied.
