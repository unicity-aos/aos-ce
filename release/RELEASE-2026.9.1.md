# AOS 2026.9.1 hotfix

Patch release following published 2026.9.0. This release combines the
credential-free Oracle bootstrap and principal metadata inspection fixes with
Astrid 2026.9.1's initialization, capsule preservation, retirement, and native
subprocess-input repairs. The Community capsule selection is unchanged.

GNU Linux, musl Linux, and macOS retain their packaged filesystem providers.
Darwin retains the signed AOS-branded filesystem app with Astrid's internal
identity. The volume format is unchanged; existing published releases and tags
are not replaced. A preservation fix cannot recreate previously lost capsule
data: reinstall affected capsules from their original trusted source if needed.

## Release dependency

The exact Astrid source commit and both GNU/Darwin and musl metadata digests
must come from authenticated published v2026.9.1 artifacts. All four direct
Astrid client dependencies must resolve to the published 2026.9.1 crates.
`scripts/validate-release-contract.py --require-release-ready` and the final
package/installer regressions must pass before tagging AOS 2026.9.1.

Signed release metadata is produced from the actual composed archives and
executable bytes. Private rehearsal hashes and identities are not production
pins. Existing tag workflows retain signature, Distro sealing, and inventory
verification. Oracle v2026.9.1 follows the published AOS patch.
