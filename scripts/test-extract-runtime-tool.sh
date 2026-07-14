#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
version=0.9.4
target=x86_64-unknown-linux-gnu
root="astrid-${version}-${target}"

mkdir -p "$work/good/$root"
printf '#!/bin/sh\nexit 0\n' > "$work/good/$root/astrid-build"
chmod 755 "$work/good/$root/astrid-build"
COPYFILE_DISABLE=1 tar -czf "$work/good.tar.gz" -C "$work/good" "$root"

"$repo_root/scripts/extract-runtime-tool.sh" \
  "$work/good.tar.gz" "$version" "$target" astrid-build "$work/bin/astrid-build"
test -x "$work/bin/astrid-build"

mkdir -p "$work/unsafe/$root"
ln -s /tmp "$work/unsafe/$root/astrid-build"
COPYFILE_DISABLE=1 tar -czf "$work/unsafe.tar.gz" -C "$work/unsafe" "$root"
if "$repo_root/scripts/extract-runtime-tool.sh" \
  "$work/unsafe.tar.gz" "$version" "$target" astrid-build "$work/bin/unsafe" \
  > /dev/null 2>&1; then
  echo "runtime tool extractor accepted a symlink" >&2
  exit 1
fi
