# AOS 2026.10.1

This maintenance release composes AOS's already-merged no-Bash shell support
and Linux installer host-setup preflight with the authenticated Astrid
2026.10.1 runtime. On minimal Linux hosts, approved foreground and background
shell commands use POSIX `sh` when Bash is absent. The installer retains the
package and gives a repair path when the packaged runtime reports missing
native prerequisites.

Astrid 2026.10.1 contains the native runtime-media isolation and the reviewed
multi-user startup, FUSE guidance, Apple Node script-loading and populated
workspace-rename fixes. AOS pins the exact public Astrid source and release
metadata in both GNU and MUSL compatibility manifests; matching registry crate
versions and package inventories are checked before publication.

## Release acceptance

The source changes and release metadata are only one gate. Before publication,
verify all six signed package archives and their contained runtime, capsule and
product hashes against the authenticated release metadata. Exercise fresh
installation and normal upgrade of the public packages on owned Apple, GNU and
MUSL fixtures. The installed journeys must cover native media denial and
permitted workspace/MCP operations, nested failure and approval recovery,
populated rename, and retained synthetic data and credential use after restart.
GNU ordinary-user first use must report missing host prerequisites, retain the
package, and recover after administrator repair; Alpine must execute approved
foreground and background shell tools without Bash.

The exact test receipt and remaining platform gaps determine release readiness.
Protected publication and channel promotion are separate approvals.
