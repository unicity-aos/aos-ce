#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 5 ]]; then
  echo "usage: $0 <runtime-archive> <runtime-version> <target> <tool> <output>" >&2
  exit 2
fi

archive=$1
version=$2
target=$3
tool=$4
output=$5
root="astrid-${version}-${target}"
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

if [[ ! -f "$archive" ]]; then
  echo "runtime archive is missing: $archive" >&2
  exit 1
fi
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "invalid runtime version: $version" >&2
  exit 1
fi
if [[ ! "$target" =~ ^[A-Za-z0-9_-]+$ || ! "$tool" =~ ^[A-Za-z0-9_-]+$ ]]; then
  echo "invalid runtime target or tool name" >&2
  exit 1
fi

python3 "$repo_root/scripts/validate-runtime-archive.py" "$archive" "$root" "$tool"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
tar -xzf "$archive" -C "$work"
source="$work/$root/$tool"
if [[ ! -f "$source" || ! -x "$source" ]]; then
  echo "runtime archive has no executable $root/$tool" >&2
  exit 1
fi
mkdir -p "$(dirname "$output")"
install -m 0755 "$source" "$output"
