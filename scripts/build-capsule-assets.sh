#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <astrid-build> <output-dir>" >&2
  exit 2
fi

builder=$1
output_dir=$2
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

if [[ "$builder" == */* ]]; then
  if [[ ! -x "$builder" ]]; then
    echo "astrid-build is missing or not executable: $builder" >&2
    exit 1
  fi
  builder=$(cd "$(dirname "$builder")" && pwd)/$(basename "$builder")
else
  builder=$(command -v "$builder" || true)
  if [[ -z "$builder" ]]; then
    echo "astrid-build is not on PATH" >&2
    exit 1
  fi
fi

mkdir -p "$output_dir"
output_dir=$(cd "$output_dir" && pwd)
if [[ -n "$(find "$output_dir" -mindepth 1 -maxdepth 1 -print -quit)" ]]; then
  echo "capsule output directory must be empty: $output_dir" >&2
  exit 1
fi

lock_hash() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$repo_root/Cargo.lock" | awk '{print $1}'
  else
    shasum -a 256 "$repo_root/Cargo.lock" | awk '{print $1}'
  fi
}

python3 "$repo_root/scripts/capsule_release.py"
before_lock=$(lock_hash)
before_source=$(git -C "$repo_root" status --porcelain --untracked-files=all -- \
  Cargo.lock Cargo.toml capsules distros release)
plan=$(mktemp)
trap 'rm -f "$plan"' EXIT
python3 "$repo_root/scripts/capsule_release.py" --print-build-plan > "$plan"

while IFS=$'\t' read -r directory package; do
  [[ -n "$directory" && -n "$package" ]]
  RUSTUP_TOOLCHAIN=1.95.0 "$builder" \
    "$repo_root/capsules/$directory" \
    --output "$output_dir"
  test -f "$output_dir/$package.capsule"
done < "$plan"

after_lock=$(lock_hash)
if [[ "$before_lock" != "$after_lock" ]]; then
  echo "capsule builds changed Cargo.lock; release builds must use the committed lock" >&2
  exit 1
fi

after_source=$(git -C "$repo_root" status --porcelain --untracked-files=all -- \
  Cargo.lock Cargo.toml capsules distros release)
if [[ "$before_source" != "$after_source" ]]; then
  echo "capsule builds changed source, manifests, WIT, distro, or release policy" >&2
  diff -u <(printf '%s\n' "$before_source") <(printf '%s\n' "$after_source") >&2 || true
  exit 1
fi

python3 "$repo_root/scripts/capsule_release.py" --artifacts "$output_dir"
