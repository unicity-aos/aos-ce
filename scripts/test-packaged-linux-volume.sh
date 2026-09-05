#!/usr/bin/env bash
set -euo pipefail

# Exercise the exact signed GNU package produced by rehearsal-sign-darwin.yml.
# This is deliberately a package consumer: it must never compile a runtime or
# invoke an installer. All state is contained below one disposable directory.
umask 077
archive=${1:-}
if [[ -z "$archive" || $# -ne 1 ]]; then
  echo "usage: $0 SIGNED_X86_64_GNU_AOS_ARCHIVE" >&2
  exit 2
fi
[[ -f "$archive" && ! -L "$archive" ]] || {
  echo "signed GNU rehearsal archive is missing or not a regular file: $archive" >&2
  exit 1
}
command -v tar >/dev/null || { echo "tar is required" >&2; exit 1; }
command -v findmnt >/dev/null || { echo "findmnt is required" >&2; exit 1; }
command -v awk >/dev/null || { echo "awk is required" >&2; exit 1; }
[[ -e /dev/fuse ]] || {
  echo "Linux packaged-volume rehearsal requires /dev/fuse" >&2
  exit 1
}
if command -v dpkg-query >/dev/null; then
  dpkg-query -W -f='${Status}' fuse3 2>/dev/null | grep -Fq 'install ok installed' || {
    echo "Linux packaged-volume rehearsal requires the fuse3 package" >&2
    exit 1
  }
elif command -v rpm >/dev/null; then
  rpm -q fuse3 >/dev/null || { echo "Linux packaged-volume rehearsal requires fuse3" >&2; exit 1; }
else
  echo "unable to verify the fuse3 package manager state" >&2
  exit 1
fi

work=$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/aos-packaged-linux-volume.XXXXXX")
mounted=0
daemon_started=0
cleanup() {
  set +e
  if (( mounted )); then
    HOME="$work/home" AOS_HOME="$work/home/.aos" ASTRID_PRINCIPAL=operator-qa \
      "$work/home/.aos/releases/2026.9.0/bin/aos" storage unmount "$work/mount" >/dev/null 2>&1
  fi
  if (( daemon_started )); then
    HOME="$work/home" AOS_HOME="$work/home/.aos" ASTRID_PRINCIPAL=operator-qa \
      "$work/home/.aos/releases/2026.9.0/bin/aos" stop >/dev/null 2>&1
  fi
  rm -rf "$work"
}
trap cleanup EXIT

mkdir -m 0700 "$work/home" "$work/extract"
expected_root=unicity-aos-2026.9.0-x86_64-unknown-linux-gnu
python3 - "$archive" "$work/extract" "$expected_root" <<'PY'
import pathlib
import sys
import tarfile

archive, destination, expected_root = sys.argv[1:]
destination = pathlib.Path(destination)
with tarfile.open(archive, "r:gz") as stream:
    members = stream.getmembers()
    if not members or not any(member.name == expected_root for member in members):
        raise SystemExit(f"archive is missing the exact root {expected_root}")
    for member in members:
        path = pathlib.PurePosixPath(member.name)
        if path.is_absolute() or ".." in path.parts:
            raise SystemExit(f"unsafe archive path: {member.name}")
        if not (member.isdir() or member.isfile()):
            raise SystemExit(f"archive member is not a regular file or directory: {member.name}")
        if not path.parts or path.parts[0] != expected_root:
            raise SystemExit(f"archive member escaped the exact root: {member.name}")
    stream.extractall(destination, filter="data")
PY

bundle="$work/extract/$expected_root"
release="$work/home/.aos/releases/2026.9.0"
mkdir -p "$work/home/.aos/releases"
chmod 0700 "$work/home/.aos" "$work/home/.aos/releases"
mv "$bundle" "$release"

for executable in \
  "$release/bin/aos" \
  "$release/runtime/bin/astrid" \
  "$release/runtime/bin/astrid-daemon" \
  "$release/runtime/bin/astrid-build" \
  "$release/runtime/bin/astrid-emit" \
  "$release/runtime/bin/astrid-storage-provider-fuse"
do
  [[ -f "$executable" && ! -L "$executable" && -x "$executable" ]] || {
    echo "package is missing executable: $executable" >&2
    exit 1
  }
done

aos="$release/bin/aos"
run_aos() {
  HOME="$work/home" AOS_HOME="$work/home/.aos" ASTRID_PRINCIPAL=operator-qa \
    "$aos" "$@"
}

# This uses the package's own product binary and signed Distro files. A
# non-admin QA identity is explicit; no unsigned or source checkout fallback is
# permitted.
run_aos distro apply --principal operator-qa --yes --offline
volume="$work/home/.aos/runtime/astrid.volume"
[[ -f "$volume" && ! -L "$volume" && -s "$volume" ]] || {
  echo "Distro Apply did not leave a non-empty stopped astrid.volume" >&2
  exit 1
}
[[ "$(stat -c '%a' "$volume" 2>/dev/null || stat -f '%Lp' "$volume")" == 600 ]] || {
  echo "stopped astrid.volume is not mode 0600" >&2
  exit 1
}
entries=$(find "$work/home/.aos/runtime" -mindepth 1 -maxdepth 1 -printf '%f\n' | sort)
[[ "$entries" == astrid.volume ]] || {
  echo "stopped runtime contains unexpected entries: $entries" >&2
  exit 1
}

run_aos start
daemon_started=1
for _ in $(seq 1 100); do
  if run_aos status --principal operator-qa --json >"$work/status.out" 2>"$work/status.err"; then
    grep -Eq 'running|Running' "$work/status.out" && break
  fi
  sleep 0.1
done
grep -Eq 'running|Running' "$work/status.out" || {
  cat "$work/status.err" >&2
  echo "packaged Astrid daemon did not reach running state" >&2
  exit 1
}

mountpoint="$work/mount"
mkdir -m 0700 "$mountpoint"
run_aos storage mount --as operator-qa --read-write "$mountpoint"
mounted=1
mount_fstype=$(findmnt -n -o FSTYPE --target "$mountpoint" || true)
[[ "$mount_fstype" == fuse* ]] || {
  echo "mountpoint is not a FUSE filesystem (type=$mount_fstype)" >&2
  exit 1
}
mountinfo=$(awk -v target="$mountpoint" '$5 == target { print; found = 1 } END { exit found ? 0 : 1 }' /proc/self/mountinfo)
[[ -n "$mountinfo" ]] || { echo "mountpoint is absent from /proc/self/mountinfo" >&2; exit 1; }

status_dirty() {
  run_aos storage status "$mountpoint" 2>"$work/status.err"
}
await_dirty() {
  local expected=$1
  local output
  for _ in $(seq 1 100); do
    if output=$(status_dirty 2>/dev/null); then
      if grep -Fq "dirty=$expected" <<<"$output"; then
        return 0
      fi
    fi
    sleep 0.1
  done
  echo "timed out waiting for dirty=$expected" >&2
  return 1
}

await_dirty false
printf 'packaged FUSE\n' > "$mountpoint/hello.txt"
[[ "$(cat "$mountpoint/hello.txt")" == 'packaged FUSE' ]]
mv "$mountpoint/hello.txt" "$mountpoint/renamed.txt"
[[ -f "$mountpoint/renamed.txt" && ! -e "$mountpoint/hello.txt" ]]
await_dirty true
run_aos storage sync "$mountpoint"
await_dirty false

rm "$mountpoint/renamed.txt"
await_dirty true
run_aos storage sync "$mountpoint"
await_dirty false

run_aos storage unmount "$mountpoint"
mounted=0
if findmnt -n --target "$mountpoint" >/dev/null 2>&1; then
  echo "FUSE mount survived unmount" >&2
  exit 1
fi
# Unmount must remove the provider's durable record and private control socket;
# the daemon's coordination directory itself remains live until stop.
registry="$work/home/.aos/run/providers/fuse"
if find "$registry" -type f \( -name '*.json' -o -name '*.sock' \) -print -quit 2>/dev/null | grep -q .; then
  echo "FUSE unmount left runtime registry/lease state" >&2
  exit 1
fi
run_aos stop
daemon_started=0
[[ "$(find "$work/home/.aos/runtime" -mindepth 1 -maxdepth 1 -printf '%f\n' | sort)" == astrid.volume ]] || {
  echo "final stopped runtime is not exactly astrid.volume" >&2
  exit 1
}
echo "packaged Linux FUSE volume rehearsal passed for explicit operator-qa"
