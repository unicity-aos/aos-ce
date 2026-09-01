#!/usr/bin/env bash

if [[ -n "${BASH_SOURCE:-}" ]]; then
    script_path=$BASH_SOURCE
else
    script_path=$0
fi
realm_root=$(cd "$(dirname "$script_path")/.." && pwd)
repo_root=$(cd "$realm_root/../.." && pwd)
toolchain_root=$(rustc --print sysroot)
cargo_home=${CARGO_HOME:-$HOME/.cargo}

export RUSTFLAGS="--cfg=getrandom_backend=\"custom\" \
--remap-path-prefix=$toolchain_root=/rust/toolchain \
--remap-path-prefix=$cargo_home=/cargo \
--remap-path-prefix=$repo_root=/src/aos-ce"
