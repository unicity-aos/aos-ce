#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
binder="$repo_root/scripts/bind-rehearsal-consumed-artifacts.sh"

bash -n "$binder"

file_mode() {
  python3 -c 'import os, sys; print(oct(os.stat(sys.argv[1]).st_mode & 0o777))' "$1"
}

make_layout() {
  local root="$1"
  mkdir -p \
    "$root/astrid-darwin" \
    "$root/astrid-x86_64-gnu" \
    "$root/astrid-aarch64-gnu" \
    "$root/aos-darwin-binary" \
    "$root/aos-x86_64-gnu-binary" \
    "$root/aos-aarch64-gnu-binary"
  printf 'darwin-runtime\n' > "$root/astrid-darwin/astrid-2026.9.0-aarch64-apple-darwin.tar.gz"
  printf 'x86_64 GNU runtime\n' > \
    "$root/astrid-x86_64-gnu/astrid-2026.9.0-x86_64-unknown-linux-gnu.tar.gz"
  printf 'aarch64 GNU runtime\n' > \
    "$root/astrid-aarch64-gnu/astrid-2026.9.0-aarch64-unknown-linux-gnu.tar.gz"
  printf 'darwin-aos\n' > "$root/aos-darwin-binary/aos"
  printf 'x86_64 GNU AOS\n' > "$root/aos-x86_64-gnu-binary/aos"
  printf 'aarch64 GNU AOS\n' > "$root/aos-aarch64-gnu-binary/aos"
  chmod 0644 \
    "$root/astrid-darwin/astrid-2026.9.0-aarch64-apple-darwin.tar.gz" \
    "$root/astrid-x86_64-gnu/astrid-2026.9.0-x86_64-unknown-linux-gnu.tar.gz" \
    "$root/astrid-aarch64-gnu/astrid-2026.9.0-aarch64-unknown-linux-gnu.tar.gz" \
    "$root/aos-darwin-binary/aos" \
    "$root/aos-x86_64-gnu-binary/aos" \
    "$root/aos-aarch64-gnu-binary/aos"
}

bind() {
  local root="$1"
  (
    cd "$root"
    bash "$binder" \
      astrid-darwin/astrid-2026.9.0-aarch64-apple-darwin.tar.gz \
      astrid-x86_64-gnu/astrid-2026.9.0-x86_64-unknown-linux-gnu.tar.gz \
      astrid-aarch64-gnu/astrid-2026.9.0-aarch64-unknown-linux-gnu.tar.gz \
      aos-darwin-binary/aos \
      aos-x86_64-gnu-binary/aos \
      aos-aarch64-gnu-binary/aos
  )
}

scratch=$(mktemp -d "${TMPDIR:-/tmp}/rehearsal-bind-XXXXXX")
trap 'rm -rf "$scratch"' EXIT

happy="$scratch/happy"
make_layout "$happy"
[[ "$(file_mode "$happy/aos-darwin-binary/aos")" == "0o644" ]]
[[ "$(file_mode "$happy/aos-x86_64-gnu-binary/aos")" == "0o644" ]]
[[ "$(file_mode "$happy/aos-aarch64-gnu-binary/aos")" == "0o644" ]]
if [[ -x "$happy/aos-darwin-binary/aos" || -x "$happy/aos-x86_64-gnu-binary/aos" || -x "$happy/aos-aarch64-gnu-binary/aos" ]]; then
  echo "fixture AOS binaries must start non-executable (mode 0644)" >&2
  exit 1
fi
bind "$happy"
[[ "$(file_mode "$happy/aos-darwin-binary/aos")" == "0o755" ]]
[[ "$(file_mode "$happy/aos-x86_64-gnu-binary/aos")" == "0o755" ]]
[[ "$(file_mode "$happy/aos-aarch64-gnu-binary/aos")" == "0o755" ]]
[[ -x "$happy/aos-darwin-binary/aos" && -x "$happy/aos-x86_64-gnu-binary/aos" && -x "$happy/aos-aarch64-gnu-binary/aos" ]]
[[ "$(file_mode "$happy/astrid-darwin/astrid-2026.9.0-aarch64-apple-darwin.tar.gz")" == "0o644" ]]
[[ "$(file_mode "$happy/astrid-x86_64-gnu/astrid-2026.9.0-x86_64-unknown-linux-gnu.tar.gz")" == "0o644" ]]
[[ "$(file_mode "$happy/astrid-aarch64-gnu/astrid-2026.9.0-aarch64-unknown-linux-gnu.tar.gz")" == "0o644" ]]

missing="$scratch/missing"
make_layout "$missing"
rm -f "$missing/aos-aarch64-gnu-binary/aos"
if bind "$missing"; then
  echo "missing AOS binary must fail closed" >&2
  exit 1
fi

symlink="$scratch/symlink"
make_layout "$symlink"
rm -f "$symlink/aos-darwin-binary/aos"
printf 'target\n' > "$symlink/aos-darwin-binary/target"
chmod 0644 "$symlink/aos-darwin-binary/target"
ln -s target "$symlink/aos-darwin-binary/aos"
if bind "$symlink"; then
  echo "symlink AOS binary must fail closed" >&2
  exit 1
fi
[[ "$(file_mode "$symlink/aos-darwin-binary/target")" == "0o644" ]]
if [[ -x "$symlink/aos-darwin-binary/target" ]]; then
  echo "rejected symlink must not chmod its target" >&2
  exit 1
fi

directory="$scratch/directory"
make_layout "$directory"
rm -f "$directory/aos-aarch64-gnu-binary/aos"
mkdir "$directory/aos-aarch64-gnu-binary/aos"
if bind "$directory"; then
  echo "directory AOS path must fail closed" >&2
  exit 1
fi

fifo="$scratch/fifo"
make_layout "$fifo"
rm -f "$fifo/aos-darwin-binary/aos"
mkfifo "$fifo/aos-darwin-binary/aos"
if bind "$fifo"; then
  echo "non-regular AOS path must fail closed" >&2
  exit 1
fi
