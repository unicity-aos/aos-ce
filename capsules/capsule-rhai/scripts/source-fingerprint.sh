#!/bin/sh
set -eu

# Resolve the repository from this script so the input paths and working
# directory cannot be selected by the operator invoking the check.
export LC_ALL=C
SCRIPT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd -P)
REPO_ROOT=$(CDPATH='' cd -- "$SCRIPT_DIR/../../.." && pwd -P)
cd "$REPO_ROOT"

listing=
while IFS= read -r path; do
    [ -n "$path" ] || continue
    if [ ! -f "$path" ]; then
        printf 'source fingerprint: missing input: %s\n' "$path" >&2
        exit 1
    fi

    line=$(shasum -a 256 -b "$path") || exit 1
    if [ -n "$listing" ]; then
        listing="${listing}
${line}"
    else
        listing=$line
    fi
done <<'EOF'
CHANGELOG.md
Cargo.toml
Cargo.lock
capsules/capsule-rhai/Cargo.toml
capsules/capsule-rhai/Capsule.toml
capsules/capsule-rhai/.cargo/config.toml
capsules/capsule-rhai/rust-toolchain.toml
capsules/capsule-rhai/src/lib.rs
capsules/capsule-rhai/src/tests.rs
capsules/capsule-rhai/scripts/verify-wasm-imports.sh
capsules/capsule-rhai/scripts/source-fingerprint.sh
capsules/capsule-rhai/README.md
capsules/capsule-rhai/DEPENDENCY-NOTES.md
EOF

# Sort complete checksum lines under the C locale, then hash the exact
# newline-terminated listing.  BUILD-VERIFICATION.md is intentionally not an
# input because it records this digest rather than contributing to the build.
printf '%s\n' "$listing" | LC_ALL=C sort | shasum -a 256 -b | awk '{print $1}'
