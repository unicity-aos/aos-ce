#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
python3 "$repo_root/scripts/validate-release-contract.py"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
target=x86_64-unknown-linux-gnu
runtime_root="$work/astrid-0.9.4-$target"
mkdir -p "$runtime_root" "$work/output"

for binary in astrid astrid-daemon astrid-build astrid-emit; do
  printf '#!/bin/sh\nexit 0\n' > "$runtime_root/$binary"
  chmod 755 "$runtime_root/$binary"
done
tar -czf "$work/runtime.tar.gz" -C "$work" "$(basename "$runtime_root")"
printf '#!/bin/sh\nexit 0\n' > "$work/aos"
chmod 755 "$work/aos"

"$repo_root/scripts/package-release.sh" \
  "$target" \
  "$work/aos" \
  "$work/runtime.tar.gz" \
  0000000000000000000000000000000000000000000000000000000000000000 \
  "$work/output"

archive="$work/output/unicity-aos-$target.tar.gz"
test -f "$archive"
tar -tzf "$archive" > "$work/files"
grep -q '/bin/aos$' "$work/files"
grep -q '/runtime/bin/astrid-daemon$' "$work/files"
grep -q '/release-manifest.json$' "$work/files"

tar -xzf "$archive" -C "$work"
manifest=$(find "$work" -path '*/release-manifest.json' -print -quit)
python3 - "$manifest" <<'PY'
import json
import pathlib
import sys

manifest = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
assert manifest["product"]["version"] == "2026.1.0"
assert manifest["runtime"]["version"] == "0.9.4"
assert manifest["runtime"]["sha256"] == "0" * 64
assert manifest["runtime"]["release_workflow_identity"] == "https://github.com/unicity-astrid/astrid/.github/workflows/release.yml@refs/tags/v0.9.4"
assert manifest["contracts"]["repository"] == "astrid-runtime/wit"
assert manifest["contracts"]["commit"] == "278dbca3e32f327d0f2358644fc86559779ba0fd"
assert manifest["contracts"]["sdk_rust_version"] == "0.7.1"
assert manifest["contracts"]["sdk_rust_commit"] == "bbbc61c8821d6c536fb25d2068b6b646e759ad35"
PY

unsafe_root="$work/unsafe-runtime"
mkdir -p "$unsafe_root/astrid-0.9.4-$target"
ln -s /tmp "$unsafe_root/astrid-0.9.4-$target/astrid"
for binary in astrid-daemon astrid-build astrid-emit; do
  printf '#!/bin/sh\nexit 0\n' > "$unsafe_root/astrid-0.9.4-$target/$binary"
  chmod 755 "$unsafe_root/astrid-0.9.4-$target/$binary"
done
tar -czf "$work/unsafe-runtime.tar.gz" -C "$unsafe_root" "astrid-0.9.4-$target"
if "$repo_root/scripts/package-release.sh" \
  "$target" \
  "$work/aos" \
  "$work/unsafe-runtime.tar.gz" \
  0000000000000000000000000000000000000000000000000000000000000000 \
  "$work/output" >/dev/null 2>&1; then
  echo "release composer accepted a symlinked runtime binary" >&2
  exit 1
fi
