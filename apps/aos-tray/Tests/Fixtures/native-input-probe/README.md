# native-input-probe

An [Astrid](https://github.com/astrid-runtime/astrid) tool capsule, scaffolded
by `astrid capsule new`.

A disposable integration fixture, not a user-facing capsule. `collect` waits
for native secret input and then checks its presence in the same invocation.
`stored` checks presence without collecting. Neither command exports a value.
Cancellation returns `input-not-completed`; an empty store returns
`secret-absent`. The test uses only synthetic input.

`text`, `empty-text`, `select`, `array`, and `empty-array` exercise ordinary
typed input. They compare the returned values inside the guest and report only
`input-matched` or `input-mismatch`. The list check preserves a comma inside one
entry rather than splitting it. Cancellation is checked separately from empty
text and empty lists. These commands require the candidate runtime's ordinary
private-input extension; a successful secret-only run does not certify them.

## Layout

| File | Purpose |
|------|---------|
| `src/lib.rs` | Bounded command parsing and SDK secret calls in a run loop. |
| `Capsule.toml` | Manifest: component, capabilities, and the tool-bus ACL. |
| `Cargo.toml` | Crate config — `cdylib` + size-optimised release profile. |
| `.cargo/config.toml` | Targets `wasm32-unknown-unknown` and sets the `getrandom` custom backend (required — without it, uuid/`HashMap` fail to link). |
| `rust-toolchain.toml` | Pins the toolchain and wasm target. |

The `wit/` directory is generated at build time; do not commit it.

## Build

```sh
astrid capsule build
```

This compiles to `wasm32-unknown-unknown` and packages a `.capsule` archive
under `dist/`.

## Run safely

Use `scripts/test-native-guest.sh` from the tray package with explicit candidate
CLI, setup helper, and built archive paths. It initializes a fresh temporary
runtime, configures a synthetic paired key, tests cancellation and successful
resumption, then clean stop/restart persistence and reconnect. It stops only
that runtime and preserves evidence. Do not install this probe into live AOS.

Authority: no network, filesystem, host process, identity, or uplink capability.
Only the provider-scoped command subscription and command-result publication
are declared. The disposable distro's required frontend role labels the fixture;
it does not add the manifest's privileged `uplink` capability.
