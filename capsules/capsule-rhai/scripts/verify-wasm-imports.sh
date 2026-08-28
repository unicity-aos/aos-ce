#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
    printf 'usage: %s path/to/module.wasm-or-capsule\n' "$0" >&2
    exit 2
fi

artifact=$1
if [[ ! -f "$artifact" ]]; then
    printf 'artifact does not exist: %s\n' "$artifact" >&2
    exit 2
fi

tmpdir=''
cleanup() {
    if [[ -n "$tmpdir" ]]; then
        rm -rf "$tmpdir"
    fi
}
trap cleanup EXIT

wasm=$artifact
if [[ "$artifact" == *.capsule ]]; then
    command -v tar >/dev/null || {
        printf 'tar is required to inspect .capsule artifacts\n' >&2
        exit 2
    }
    tmpdir=$(mktemp -d "${TMPDIR:-/tmp}/aos-rhai-imports.XXXXXX")
    tar -xzf "$artifact" -C "$tmpdir"
    wasm_files=()
    while IFS= read -r path; do
        wasm_files[${#wasm_files[@]}]=$path
    done < <(find "$tmpdir" -type f -name '*.wasm' -print)
    if [[ ${#wasm_files[@]} -ne 1 ]]; then
        printf 'expected exactly one wasm payload, found %s\n' "${#wasm_files[@]}" >&2
        exit 1
    fi
    wasm=${wasm_files[0]}
fi

command -v wasm-tools >/dev/null || {
    printf 'wasm-tools is required to inspect imports\n' >&2
    exit 2
}

imports=$(mktemp "${TMPDIR:-/tmp}/aos-rhai-import-list.XXXXXX")
expected=$(mktemp "${TMPDIR:-/tmp}/aos-rhai-import-expected.XXXXXX")
modules=$(mktemp "${TMPDIR:-/tmp}/aos-rhai-import-modules.XXXXXX")
expected_modules=$(mktemp "${TMPDIR:-/tmp}/aos-rhai-import-modules-expected.XXXXXX")
printed=$(mktemp "${TMPDIR:-/tmp}/aos-rhai-import-print.XXXXXX")
trap 'rm -f "$imports" "$expected" "$modules" "$expected_modules" "$printed"; cleanup' EXIT

wasm-tools print "$wasm" >"$printed"

# Component-level imports and core linker shims have different S-expression
# shapes.  The exact two-string form below is the host function surface; omit
# the empty-module linker shims (which appear as /0, /1, ...).
sed -n 's/^[[:space:]]*(import "\([^"]*\)" "\([^"]*\)".*/\1\/\2/p' "$printed" \
    | grep -v '^/' \
    | sort -u >"$imports"

# Check the module-level surface too.  This catches a newly introduced
# component import even when it does not use the two-string core-import form.
sed -n 's/^[[:space:]]*(import "\([^"]*\)".*/\1/p' "$printed" \
    | grep -v '^$' \
    | sort -u >"$modules"

cat >"$expected" <<'EOF'
astrid:ipc/host@1.0.0/publish
astrid:sys/host@1.0.0/log
astrid:sys/host@1.0.0/random-bytes
EOF

cat >"$expected_modules" <<'EOF'
astrid:guest/lifecycle@1.0.0
astrid:ipc/host@1.0.0
astrid:sys/host@1.0.0
capsule-result
EOF

if grep -Eiq '\(import "[^"]*(fs|filesystem|net|network|process|clock|time|credential|credentials)[^"]*"' "$printed"; then
    printf 'forbidden host import found:\n' >&2
    grep -Ei '\(import "[^"]*(fs|filesystem|net|network|process|clock|time|credential|credentials)[^"]*"' "$printed" >&2
    exit 1
fi

if ! diff -u "$expected" "$imports"; then
    printf 'wasm imports differ from the aos-rhai allowlist\n' >&2
    exit 1
fi

if ! diff -u "$expected_modules" "$modules"; then
    printf 'wasm import modules differ from the aos-rhai allowlist\n' >&2
    exit 1
fi

printf 'verified %s: only Astrid SDK IPC/sys plumbing imports present\n' "$artifact"
