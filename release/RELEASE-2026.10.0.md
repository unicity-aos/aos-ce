# AOS 2026.10.0

This monthly release adds a shared software-update inventory to Command Center
and `aos console`, including verified AOS/Oracle updates, principal-scoped
capsule discovery and explicit activation status. Native hook routing preserves
authenticated policy decisions; the Oracle layer still owns client-specific
event translation. Fast provider streaming keeps turn ownership and ordering.

First Community initialization grants its default fleet as part of the bounded
runtime initialization. Cold MCP discovery waits for delayed providers through
its collection deadline, and principal discovery selects the product workspace.
Regular Finder metadata no longer causes false shutdown failure. Older macOS
hosts can use the CLI without unsupported desktop or FSKit components; native
mounting still requires the app's supported OS version and extension approval.

## Preparation boundary

The intended runtime is Astrid v2026.10.0. Until its separately authorized release
publishes authenticated metadata, the compatibility files retain valid v2026.9.4
provenance with `release-ready` and `upgrade-self-heal-ready` gates closed.
This is not a ready-to-publish old-runtime fallback. Replace GNU/Darwin and MUSL
runtime provenance together, update compiled runtime-version declarations and
exact Rust build dependencies, and validate the composed package before opening
the gates. Installed-runtime compatibility remains a minimum requirement, not an
exact-version ceiling.

The selected runtime-code rehearsal already exercises real signature-enabled
package installation, legacy import, capsule execution and restart persistence,
Oracle MCP calls and known-work metrics. Final product-version composition and
production identity are separate evidence boundaries. QA trust substitution must
preserve verification logic; it is not production attestation.

GNU and MUSL Linux on ARM64/x86_64 and Darwin on Apple Silicon/Intel remain the
intended matrix. Windows archives remain outside this product release. Record
native execution versus build/CI evidence explicitly; unsupported hosts must not
be represented as executed platforms.

Merge does not authorize a tag, publication or channel promotion.
