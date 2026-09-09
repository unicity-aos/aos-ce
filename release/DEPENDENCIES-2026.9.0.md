# Release dependency version inventory

Baseline: `2026.1.3`. Candidate: final `2026.9.0` release PR Cargo.lock.

Includes direct, transitive, workspace, test, and platform-specific packages. Inclusion does not mean every package ships in every binary.

## Updated (24)

| Package | Previous | Candidate |
| --- | --- | --- |
| `astrid-core` | 0.10.4 | 2026.9.0 |
| `astrid-crypto` | 0.10.4 | 2026.9.0 |
| `astrid-sdk` | 0.5.3, 0.7.1 | 0.7.1 |
| `astrid-sdk-macros` | 0.5.3, 0.7.1 | 0.7.1 |
| `astrid-sys` | 0.5.3, 0.7.1 | 0.7.1 |
| `astrid-types` | 0.10.4, 0.4.0, 0.7.0 | 0.7.0, 2026.9.0 |
| `astrid-uplink` | 0.10.4 | 2026.9.0 |
| `base64` | 0.22.1 | 0.23.1 |
| `getrandom` | 0.4.3 | 0.2.17, 0.3.4, 0.4.3 |
| `hashbrown` | 0.16.1, 0.17.1 | 0.14.5, 0.16.1, 0.17.1 |
| `js-sys` | 0.3.103 | 0.3.105 |
| `r-efi` | 6.0.0 | 5.3.0, 6.0.0 |
| `serde_spanned` | 0.6.9 | 0.6.9, 1.1.1 |
| `syn` | 2.0.118 | 2.0.118, 3.0.5 |
| `toml` | 0.8.23 | 0.8.23, 1.1.5+spec-1.1.0 |
| `toml_parser` | 1.1.2+spec-1.1.0 | 1.1.3+spec-1.1.0 |
| `unicity-aos-bootstrap` | 2026.1.3 | 2026.9.0 |
| `uuid` | 1.23.5 | 1.25.0 |
| `uuid-rng-internal` | 1.23.5 | 1.26.0 |
| `wasm-bindgen` | 0.2.126 | 0.2.128 |
| `wasm-bindgen-macro` | 0.2.126 | 0.2.128 |
| `wasm-bindgen-macro-support` | 0.2.126 | 0.2.128 |
| `wasm-bindgen-shared` | 0.2.126 | 0.2.128 |
| `wit-bindgen` | 0.54.0 | 0.54.0, 0.57.1 |

## Added (60)

| Package | Previous | Candidate |
| --- | --- | --- |
| `adaptive-shell` | — | 0.1.0 |
| `ahash` | — | 0.8.12 |
| `aos-hook-adapter-oracle` | — | 0.1.0 |
| `aos-mcp` | — | 0.1.0 |
| `aos-mcp-broker` | — | 0.1.0 |
| `aos-meta-harness` | — | 0.1.0 |
| `aos-rhai` | — | 0.1.0 |
| `astrid-config` | — | 2026.9.0 |
| `astrid-events` | — | 2026.9.0 |
| `astrid-runtime` | — | 2026.9.0 |
| `capsule-surface-catalog` | — | 0.1.0 |
| `capsule-surface-model` | — | 0.1.0 |
| `capsule-workspace-recipes` | — | 0.1.0 |
| `const-random` | — | 0.1.18 |
| `const-random-macro` | — | 0.1.16 |
| `crossbeam-utils` | — | 0.8.23 |
| `crunchy` | — | 0.2.4 |
| `dashmap` | — | 6.2.1 |
| `directories` | — | 6.0.0 |
| `dirs-sys` | — | 0.5.0 |
| `dispatch2` | — | 0.3.1 |
| `errno` | — | 0.3.14 |
| `futures` | — | 0.3.32 |
| `futures-executor` | — | 0.3.32 |
| `futures-io` | — | 0.3.34 |
| `futures-macro` | — | 0.3.32 |
| `futures-sink` | — | 0.3.34 |
| `libredox` | — | 0.1.23 |
| `lock_api` | — | 0.4.14 |
| `metrics` | — | 0.24.6 |
| `nix` | — | 0.31.3 |
| `objc2` | — | 0.6.4 |
| `objc2-app-kit` | — | 0.3.2 |
| `objc2-core-foundation` | — | 0.3.2 |
| `objc2-encode` | — | 4.1.0 |
| `objc2-foundation` | — | 0.3.2 |
| `option-ext` | — | 0.2.0 |
| `parking_lot` | — | 0.12.5 |
| `parking_lot_core` | — | 0.9.12 |
| `pin-utils` | — | 0.1.0 |
| `portable-atomic` | — | 1.15.0 |
| `rapidhash` | — | 4.5.1 |
| `redox_syscall` | — | 0.5.18 |
| `redox_users` | — | 0.5.2 |
| `rhai` | — | 1.26.0 |
| `rhai_codegen` | — | 3.2.0 |
| `scopeguard` | — | 1.2.0 |
| `signal-hook-registry` | — | 1.4.8 |
| `smartstring` | — | 1.0.1 |
| `static_assertions` | — | 1.1.0 |
| `thin-vec` | — | 0.2.19 |
| `tiny-keccak` | — | 2.0.2 |
| `toml_writer` | — | 1.1.2+spec-1.1.0 |
| `version_check` | — | 0.9.5 |
| `wasip2` | — | 1.0.4+wasi-0.2.12 |
| `wasm-bindgen-futures` | — | 0.4.78 |
| `wasmtimer` | — | 0.4.3 |
| `web-time` | — | 1.1.0 |
| `zerocopy` | — | 0.8.56 |
| `zerocopy-derive` | — | 0.8.56 |

## Removed (20)

| Package | Previous | Candidate |
| --- | --- | --- |
| `aho-corasick` | 1.1.4 | — |
| `aos-telegram` | 0.1.0 | — |
| `bytemuck` | 1.25.1 | — |
| `either` | 1.16.0 | — |
| `extism-convert` | 1.30.0 | — |
| `extism-convert-macros` | 1.30.0 | — |
| `extism-manifest` | 1.30.0 | — |
| `extism-pdk` | 1.4.1 | — |
| `extism-pdk-derive` | 1.4.1 | — |
| `itertools` | 0.14.0 | — |
| `manyhow` | 0.11.4 | — |
| `manyhow-macros` | 0.11.4 | — |
| `proc-macro-utils` | 0.10.0 | — |
| `prost` | 0.14.4 | — |
| `prost-derive` | 0.14.4 | — |
| `regex` | 1.13.0 | — |
| `regex-automata` | 0.4.15 | — |
| `regex-syntax` | 0.8.11 | — |
| `rmp` | 0.8.15 | — |
| `rmp-serde` | 1.3.1 | — |

Unchanged version sets are omitted. Source and checksum changes remain visible in Cargo.lock.
