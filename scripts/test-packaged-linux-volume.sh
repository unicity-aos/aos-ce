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
command -v b3sum >/dev/null || { echo "b3sum is required to verify the signed rehearsal identity" >&2; exit 1; }
fusermount3=$(command -v fusermount3) || { echo "fusermount3 is required" >&2; exit 1; }
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
cleanup() {
  local status=$?
  local unsafe=0
  set +e
  # Inspect the kernel's actual mount table instead of trusting our flags: a
  # failed mount/unmount must never be hidden by an early shell exit.
  if mount_is_active; then
    if [[ -x "${aos:-}" ]]; then
      run_aos storage unmount "$mountpoint" >/dev/null 2>&1 || unsafe=1
    else
      unsafe=1
    fi
    mount_is_active && unsafe=1
  fi
  if [[ -x "${aos:-}" ]]; then
    # Stop is idempotent for a stopped disposable daemon.  A failure is only
    # fatal when a process under this disposable AOS home is still alive.
    local pid=''
    if [[ -f "$work/home/.aos/run/system.pid" ]]; then
      pid=$(<"$work/home/.aos/run/system.pid")
    fi
    run_aos stop >/dev/null 2>&1 || true
    [[ "$pid" =~ ^[1-9][0-9]*$ ]] && kill -0 "$pid" 2>/dev/null && unsafe=1
  fi
  if (( unsafe )); then
    echo "unsafe cleanup; preserving disposable evidence at $work" >&2
    exit 1
  fi
  if ! rm -rf "$work"; then
    echo "unable to remove disposable evidence; preserving $work" >&2
    exit 1
  fi
  exit "$status"
}
trap cleanup EXIT

mkdir -m 0700 "$work/home" "$work/extract"
{
  printf 'runner_image=%s\n' "${ImageOS:-unknown}"
  printf 'runner_version=%s\n' "${ImageVersion:-unknown}"
  printf 'uname=%s\n' "$(uname -a)"
  printf 'fusermount3=%s\n' "$fusermount3"
  printf 'fusermount3_mode=%s\n' "$(stat -c '%a' "$fusermount3" 2>/dev/null || stat -f '%Lp' "$fusermount3")"
  "$fusermount3" --version 2>&1 || true
} | tee "$work/runner-fuse.txt"
expected_root=unicity-aos-2026.9.0-x86_64-unknown-linux-gnu
# Bind the exact package bytes to the REHEARSAL-ONLY identity and checksum
# manifests emitted by compose-and-sign.  Downloading an artifact is not an
# identity check: reject renamed archives, mismatched hashes, and malformed
# manifest records before extraction or execution.
artifact_dir=$(dirname "$archive")
identity="$artifact_dir/REHEARSAL-ONLY-identity.json"
blake_manifest="$artifact_dir/REHEARSAL-BLAKE3SUMS.txt"
sha_manifest="$artifact_dir/REHEARSAL-SHA256SUMS.txt"
for manifest in "$identity" "$blake_manifest" "$sha_manifest"; do
  [[ -f "$manifest" && ! -L "$manifest" ]] || {
    echo "signed rehearsal artifact is missing identity/checksum manifest: $manifest" >&2
    exit 1
  }
done
python3 - "$identity" "$blake_manifest" "$sha_manifest" "$archive" <<'PY'
import hashlib
import json
import os
import pathlib
import subprocess
import sys

identity_path, blake_path, sha_path, archive = map(pathlib.Path, sys.argv[1:])
name = archive.name
identity = json.loads(identity_path.read_text(encoding="utf-8"))
if identity.get("scope") != "REHEARSAL-ONLY" or identity.get("publication_allowed") is not False:
    raise SystemExit("archive identity is not explicitly rehearsal-only")
expected_source = os.environ.get("GITHUB_SHA", "")
if expected_source and identity.get("aos", {}).get("source_commit") != expected_source:
    raise SystemExit("archive identity does not bind the exact AOS workflow commit")
record = identity.get("signed_archive_digests", {}).get("x86_64-unknown-linux-gnu")
if not isinstance(record, dict) or record.get("archive") != name:
    raise SystemExit("archive name does not match the signed rehearsal identity")
expected_blake = record.get("blake3_rehearsal_only", "")
expected_sha = record.get("sha256_rehearsal_only", "")
if not expected_blake.startswith("blake3:") or not expected_sha.startswith("sha256:"):
    raise SystemExit("identity is missing canonical archive digests")
def manifest_digest(path, expected_name):
    rows = [line.split() for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]
    matches = [row[0] for row in rows if len(row) == 2 and row[1] == expected_name]
    if len(matches) != 1:
        raise SystemExit(f"checksum manifest does not contain exactly one {expected_name} record")
    return matches[0]
if manifest_digest(blake_path, name) != expected_blake.removeprefix("blake3:"):
    raise SystemExit("BLAKE3 manifest disagrees with signed identity")
if manifest_digest(sha_path, name) != expected_sha.removeprefix("sha256:"):
    raise SystemExit("SHA256 manifest disagrees with signed identity")
actual_sha = hashlib.sha256(archive.read_bytes()).hexdigest()
if actual_sha != expected_sha.removeprefix("sha256:"):
    raise SystemExit("archive SHA256 does not match signed identity")
actual_blake = subprocess.check_output(["b3sum", "--", str(archive)], text=True).split()[0]
if actual_blake != expected_blake.removeprefix("blake3:"):
    raise SystemExit("archive BLAKE3 does not match signed identity")
PY
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
python3 - "$bundle/Distro.toml" <<'PY'
import pathlib
import sys
import tomllib

manifest = pathlib.Path(sys.argv[1])
try:
    capsules = tomllib.loads(manifest.read_text(encoding="utf-8")).get("capsule", [])
except (OSError, tomllib.TOMLDecodeError) as error:
    raise SystemExit(f"unable to parse packaged Distro.toml: {error}")
names = [item.get("name") for item in capsules]
if len(names) != 22 or len(set(names)) != 22 or any(not name for name in names):
    raise SystemExit("packaged Distro.toml must declare exactly 22 unique capsules")
PY
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
  HOME="$work/home" AOS_HOME="$work/home/.aos" \
    ASTRID_PRINCIPAL=operator-qa ASTRID_VAR_OPENAI_API_KEY=release-gate-not-a-real-key \
    "$aos" "$@"
}
run_default() {
  HOME="$work/home" AOS_HOME="$work/home/.aos" ASTRID_PRINCIPAL=default \
    "$aos" --principal default "$@"
}
active_receipt="$work/home/.aos/receipts/unicity-ce.active.json"
runtime_distro_lock="$work/home/.aos/runtime/home/operator-qa/.config/distro.lock"
mount_is_active() {
  [[ -n "${mountpoint:-}" ]] && findmnt -n --mountpoint "$mountpoint" >/dev/null 2>&1
}

# This uses the package's own product binary and signed Distro files. A
# non-admin QA identity is explicit; no unsigned or source checkout fallback is
# permitted.
# Start the packaged runtime first, then have the authenticated default
# principal mint the explicit non-admin operator-qa key/profile.  This proves
# that Distro Apply cannot silently fall back to an anonymous identity.
run_default start
run_default agent create operator-qa --group agent --yes
operator_show=$(run_aos agent show operator-qa --format json)
python3 - "$operator_show" <<'PY'
import json
import sys

try:
    record = json.loads(sys.argv[1])
except (IndexError, json.JSONDecodeError) as error:
    raise SystemExit(f"operator-qa identity output is not JSON: {error}")
if not isinstance(record, dict):
    raise SystemExit("operator-qa identity output is not an object")
principal = record.get("principal", record.get("id", record.get("name")))
if principal != "operator-qa":
    raise SystemExit(f"authenticated agent show returned unexpected principal: {principal!r}")
PY

# A partial install is the only expected nonzero result.  Retry at most three
# bounded passes (the documented 10+10+2 convergence) and reject every other
# failure.  A partial pass must leave no activation receipt/lock, so the next
# pass cannot be mistaken for a fresh completed install.
apply_succeeded=0
for pass in 1 2 3; do
  apply_output="$work/distro-apply-${pass}.log"
  set +e
  run_aos distro apply --principal operator-qa --yes --offline >"$apply_output" 2>&1
  apply_status=$?
  set -e
  if (( apply_status == 0 )); then
    apply_succeeded=1
    break
  fi
  cat "$apply_output" >&2
  if ! grep -Fq 'Installation incomplete:' "$apply_output"; then
    echo "Distro Apply failed without the documented partial-install marker" >&2
    exit "$apply_status"
  fi
  # Sealed release/Distro.lock is package input, not installation progress.
  # Probe the exact principal runtime lock and completion receipt instead.
  if [[ -e "$runtime_distro_lock" || -L "$runtime_distro_lock" ]]; then
    echo "partial Distro Apply wrote a lock; refusing to retry" >&2
    exit 1
  fi
  [[ ! -e "$active_receipt" && ! -L "$active_receipt" ]] || {
    echo "partial Distro Apply wrote an activation receipt" >&2
    exit 1
  }
done
(( apply_succeeded == 1 )) || {
  echo "Distro Apply did not converge after the bounded 10+10+2 passes" >&2
  exit 1
}
receipt="$active_receipt"
[[ -f "$receipt" && ! -L "$receipt" ]] || {
  echo "Distro Apply did not persist its success receipt" >&2
  exit 1
}
python3 - "$receipt" <<'PY'
import json
import pathlib
import sys

receipt = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
if receipt.get("active") is not True or receipt.get("cutover_complete") is not True:
    raise SystemExit("Distro Apply receipt is not an active completed receipt")
if receipt.get("distro_id") != "unicity-ce" or receipt.get("principal") != "operator-qa":
    raise SystemExit("Distro Apply receipt is bound to the wrong distro or principal")
PY
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
# The running projection is the authority for the final 22-member lock and
# grants.  `ps` is queried through the packaged CLI, never by reading an
# untrusted manifest or synthesizing a grant set in the harness.
capsules_json=$(run_aos ps --format json)
python3 - "$capsules_json" <<'PY'
import json
import sys

rows = json.loads(sys.argv[1])
if not isinstance(rows, list) or len(rows) != 22:
    raise SystemExit(f"expected exactly 22 ready capsules, got {len(rows) if isinstance(rows, list) else type(rows)}")
if any(row.get("state") != "ready" for row in rows):
    raise SystemExit("one or more Distro capsules is not ready")
PY
if [[ ! -f "$runtime_distro_lock" || -L "$runtime_distro_lock" ]]; then
  echo "completed Distro Apply did not leave a durable Distro.lock" >&2
  exit 1
fi

mountpoint="$work/mount"
mkdir -m 0700 "$mountpoint"
run_aos storage mount --as operator-qa --read-write "$mountpoint"
mount_fstype=$(findmnt -n -o FSTYPE --mountpoint "$mountpoint" || true)
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
if mount_is_active; then
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
daemon_pid=''
if [[ -f "$work/home/.aos/run/system.pid" ]]; then
  daemon_pid=$(<"$work/home/.aos/run/system.pid")
fi
run_aos stop
if [[ "$daemon_pid" =~ ^[1-9][0-9]*$ ]]; then
  for _ in $(seq 1 100); do
    kill -0 "$daemon_pid" 2>/dev/null || break
    sleep 0.1
  done
  if kill -0 "$daemon_pid" 2>/dev/null; then
    echo "packaged Astrid daemon remained alive after stop" >&2
    exit 1
  fi
fi
[[ "$(find "$work/home/.aos/runtime" -mindepth 1 -maxdepth 1 -printf '%f\n' | sort)" == astrid.volume ]] || {
  echo "final stopped runtime is not exactly astrid.volume" >&2
  exit 1
}
echo "packaged Linux FUSE volume rehearsal passed for explicit operator-qa"
