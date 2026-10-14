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

## Release preparation and acceptance

This stable preparation selects authenticated public Astrid v2026.10.0
metadata and exact registry dependencies. The regenerated Rust lockfile and
final composed AOS package acceptance must pass before publication.
Installed-runtime compatibility remains a minimum requirement, not an
exact-version ceiling.

Published AOS RC7 with Astrid RC6 passed the named Apple, GNU Linux and MUSL
installation and nested-failure recovery journeys, including calls resumed
after approval. Real GNU/MUSL CLI upgrades retained two principals' data and
usable credentials across two daemon restarts. Packaged capsule build,
installation, hot update, configuration, restart persistence and removal also
passed. These results establish candidate behavior; they do not substitute for
acceptance of the final AOS stable packages.

The final Astrid stable Apple archive passed all eleven checks in the unchanged
canonical FSKit runner, with exact host restoration verified afterward. Final
GNU/MUSL CI archives passed real filesystem persistence and restart checks.
All six public stable archives passed signature, digest, size and archive
contract checks; the Apple archive matches the certified bytes. Standalone
public stable runtime nested-trap and approval-resume recovery passed on Apple,
GNU Linux and MUSL with the unchanged RC7 broker. Final composed AOS
installation and upgrade acceptance remain release gates.

Published Oracle 2026.10.1 fixes combined AOS/Astrid version selection and the
Codex launcher's dependency on a global b3sum executable. Its authenticated
public packages passed the named Apple/GNU/MUSL startup and real-tool checks.
The native-client trials used deterministic local model endpoints; they do not
certify fresh proprietary-provider login.

If an Oracle installer is forcibly killed after starting the runtime, its
runtime can retain the installation lock (Oracle #109). Once no installer is
active, an ordinary `aos stop` followed by installer retry recovers without
removing lock files or resetting data; that recovery and repeat install passed.
This remains a known hard-interruption limitation, not a whole-product rollback
guarantee.

The AOS installer shares checksum-verified Sigstore verifier bytes with Oracle
and tolerates healthy slow downloads without bypassing verification. A changed
plugin generation requires a new client session. Storage allocated-space
retention remains a known limitation. Whole-machine reboot, physical Intel
execution and separate OS-user journeys remain unverified; daemon restarts
and cross-compilation are not evidence for those cases.

GNU and MUSL Linux on ARM64/x86_64 and Darwin on Apple Silicon/Intel remain the
intended matrix. Windows archives remain outside this product release. Record
native execution versus build/CI evidence explicitly; unsupported hosts must not
be represented as executed platforms.

Merge does not authorize a tag, publication or channel promotion.
