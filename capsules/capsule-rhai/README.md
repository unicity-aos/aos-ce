# AOS Rhai capsule

`aos-rhai` is a small, user-space script runtime for recipes and surface
behaviour.  The `evaluate_script` tool accepts a script, a JSON value exposed as
`input`, an optional named profile, and optional per-invocation restrictions.
The `list_script_profiles` tool returns the profile limits and language options.

Every invocation creates a fresh Rhai engine and scope.  Profile defaults are
compiled into the capsule; request fields can only make those defaults stricter.
There is no environment-variable or process-global configuration.  The hard
ceiling is also compiled into the capsule and is never widened by a request.

The component has no manifest capabilities.  The generated component imports
only Astrid SDK plumbing (`astrid:ipc/host.publish`,
`astrid:sys/host.log`, and `astrid:sys/host.random-bytes`); those imports are
entry-point plumbing, not script APIs.  Scripts receive no filesystem,
network, process, clock, IPC, or credential primitives, and no random function
is registered.  Rhai 1.26 enables `ahash`'s compile-time-rng feature; this
capsule also enables `ahash/runtime-rng` so production engines derive fresh
hash-table keys from the host random-bytes import.  Runtime entropy is not
embedded in the WASM, so clean release builds remain byte-for-byte stable.
`sleep` and Rhai's parser-level `eval` are disabled explicitly.
Rhai errors are reduced to stable error codes without returning source text or
untrusted exception details.  Operation, call-depth, expression-depth,
variable, collection, string, script, input, output, and cooperative
cancellation limits are bounded before and during evaluation.  The
`no_module` build feature and import-free package registration keep nested
source and module loading outside the contract.

Run `scripts/verify-wasm-imports.sh path/to/aos_rhai.wasm` (or point it at a
`.capsule`) to validate the generated import allowlist.  The check fails on an
unexpected import or any filesystem/network/process/time/clock/credential
name, so an accidental host primitive cannot silently enter the artifact.

This is not an authorization layer and does not replace host policy, grants,
or the WASM sandbox.  A future effectful composition must provide its own
reviewed adapter and manifest capabilities.  Cancellation supplied in the
request is cooperative (an operation threshold); host interruption and memory
containment remain runtime responsibilities.

Rhai currently depends on the unmaintained `smartstring 1.0.1`
(`RUSTSEC-2026-0249`).  No compatible maintained replacement is available
through Rhai 1.26, so this is an explicitly accepted maintenance residual, not
a vulnerability fix.  Re-check the advisory and Rhai's dependency graph on
every upgrade, and remove the exception before selecting this capsule for a
release allowlist.  The exact review and tracking note is in
`DEPENDENCY-NOTES.md`.

The package targets `wasm32-unknown-unknown` and pins Rust 1.94.0 in its local
`rust-toolchain.toml`.  From the repository root, enter the capsule directory
before running the fingerprint and build commands so the repository-wide Rust
1.95.0 pin cannot silently change this capsule's bytes:

```text
cd capsules/capsule-rhai
./scripts/source-fingerprint.sh
cargo build --release
CARGO_TARGET_DIR=target aos capsule build . --output <output-dir>
```

AOS distro/release selection and activation remain a later integration
concern.
