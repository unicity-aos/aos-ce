#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 6 ]]; then
  echo "usage: bind-rehearsal-consumed-artifacts.sh DARWIN_RUNTIME X86_64_GNU_RUNTIME AARCH64_GNU_RUNTIME DARWIN_AOS X86_64_GNU_AOS AARCH64_GNU_AOS" >&2
  exit 1
fi

require_regular_file() {
  local path="$1"
  if [[ -L "$path" ]]; then
    echo "rehearsal artifact must not be a symlink: $path" >&2
    exit 1
  fi
  if [[ ! -f "$path" ]]; then
    echo "rehearsal artifact is missing or not a regular file: $path" >&2
    exit 1
  fi
}

print_stat() {
  local path="$1"
  echo "rehearsal artifact stat: $path"
  if stat --version >/dev/null 2>&1; then
    stat --printf '  name=%n mode=%a uid=%u gid=%g type=%F\n' -- "$path"
  else
    stat -f '  name=%N mode=%Lp uid=%u gid=%g type=%HT' -- "$path"
  fi
}

DARWIN_RUNTIME_ARCHIVE="$1"
X86_64_GNU_RUNTIME_ARCHIVE="$2"
AARCH64_GNU_RUNTIME_ARCHIVE="$3"
DARWIN_AOS_BINARY="$4"
X86_64_GNU_AOS_BINARY="$5"
AARCH64_GNU_AOS_BINARY="$6"

require_regular_file "$DARWIN_RUNTIME_ARCHIVE"
require_regular_file "$X86_64_GNU_RUNTIME_ARCHIVE"
require_regular_file "$AARCH64_GNU_RUNTIME_ARCHIVE"
require_regular_file "$DARWIN_AOS_BINARY"
require_regular_file "$X86_64_GNU_AOS_BINARY"
require_regular_file "$AARCH64_GNU_AOS_BINARY"
echo "rehearsal consumed artifacts before chmod"
print_stat "$DARWIN_RUNTIME_ARCHIVE"
print_stat "$X86_64_GNU_RUNTIME_ARCHIVE"
print_stat "$AARCH64_GNU_RUNTIME_ARCHIVE"
print_stat "$DARWIN_AOS_BINARY"
print_stat "$X86_64_GNU_AOS_BINARY"
print_stat "$AARCH64_GNU_AOS_BINARY"
chmod 0755 \
  "$DARWIN_AOS_BINARY" \
  "$X86_64_GNU_AOS_BINARY" \
  "$AARCH64_GNU_AOS_BINARY"
echo "rehearsal AOS binaries after chmod 0755"
print_stat "$DARWIN_AOS_BINARY"
print_stat "$X86_64_GNU_AOS_BINARY"
print_stat "$AARCH64_GNU_AOS_BINARY"
if [[ ! -x "$DARWIN_AOS_BINARY" || ! -x "$X86_64_GNU_AOS_BINARY" || ! -x "$AARCH64_GNU_AOS_BINARY" ]]; then
  echo "rehearsal AOS binaries are not executable after chmod 0755" >&2
  exit 1
fi
