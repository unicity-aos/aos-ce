# Runtime recovery qualification (AOS #206)

The current AOS 2026.9.3 pin names Astrid 2026.9.4. These are distinct product
and runtime versions. That runtime predates Astrid #2010, so it cannot satisfy
the new release recovery gate. The existing pin is retained as accurate
published provenance until a fixed signed release is available.

Astrid 2026.9.5 is the prepared maintenance candidate. Its source must include
34bfab971cb780815b50ae9a51dc23d6646161d9. AOS release builds now depend on a
Linux/macOS qualification job that checks the exact pinned source commit,
clean checkout, version, fix ancestry and all eight recovery regressions. Missing
or ignored tests fail qualification. Existing artifact authenticity and archive
validation still apply; passing source tests alone does not authenticate a binary.

## Bind the published release

1. Release Astrid through its ordinary signed six-target workflow after source
   qualification and CI. Verify the exact tag identities on both the immutable
   `astrid-2026.9.5-release.toml` metadata and the musl extension. Verify archive
   signatures, metadata digests, source commit, macOS notarization and native
   storage certification. No hashes are supplied here for unbuilt artifacts.
2. Update `release/runtime-compatibility.toml` and
   `release/runtime-musl-compatibility.toml` together using the authenticated
   published source commit and actual metadata BLAKE3 digests. Update version,
   tag, requirement and exact workflow identities. Keep readiness false until
   the final artifact rehearsals pass.
3. Align `release/runtime-command-surface.toml`, the compiled
   `ASTRID_RUNTIME_VERSION` in `crates/unicity-aos-bootstrap/src/distro_trust.rs`,
   and `distros/community/unicity-ce/Distro.toml`. If the Astrid Rust dependency
   versions change, update the four workspace dependencies and lockfile only
   after those crates are published. Update version assertions and release notes.
   The composer and installer already preserve the required FUSE/FSKit
   membership for the 2026.9.5 runtime contract.
4. Run `python3 scripts/runtime_recovery.py /path/to/exact/astrid/source`,
   `bash scripts/test-pinned-runtime-command-surface.sh`, the release-contract,
   packaging, install and upgrade/self-heal tests, and the ordinary native
   filesystem rehearsals against the final signed candidate. Source tests
   exercise fuel exhaustion, epoch expiry, cancellation, guest traps, fixed-size
   replacement, warm reuse and fail-closed dispatcher behavior.
5. Rehearse installation and policy activation in a disposable endpoint. Trigger
   an interrupted enforcer call, verify that it denies that operation, and verify
   that the next bounded call can execute without `cannot enter component
   instance`. Verify the final released binary, not only this source fixture.

This change recovers interrupted runtime instances. It does not raise the
Codewall enforcer's CPU quota or fix its initial policy-activation fuel demand
(Codewall #43). Successful end-to-end installation still depends on that work
and the separately reported Codewall installation-sequence issues.

The original failed endpoint is evidence. Do not repair or restart it during
source qualification; use isolated fixture homes and worktrees instead.
