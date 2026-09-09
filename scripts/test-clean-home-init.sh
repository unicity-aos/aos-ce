#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
  echo "usage: $0 <extracted-product-bundle> [prebuilt-provenance-probe]" >&2
  exit 2
fi

bundle=$(cd "$1" && pwd -P)
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
if [[ $# -eq 2 ]]; then
  # Release CI builds this in the same container as AOS. Do not compile into
  # that container's root-owned target tree from the unprivileged host runner.
  provenance_probe=$(cd "$(dirname "$2")" && pwd -P)/$(basename "$2")
  [[ -f "$provenance_probe" && -x "$provenance_probe" && ! -L "$provenance_probe" ]] || {
    echo "invalid prebuilt provenance probe: $provenance_probe" >&2
    exit 1
  }
else
  cargo build --locked --manifest-path "$repo_root/Cargo.toml" \
    -p unicity-aos-bootstrap --example init_provenance
  target_dir=$(cargo metadata --locked --manifest-path "$repo_root/Cargo.toml" \
    --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')
  provenance_probe="$target_dir/debug/examples/init_provenance"
fi
for required in bin/aos runtime/bin/astrid runtime/bin/astrid-daemon Distro.toml capsule-assets.txt; do
  [[ -f "$bundle/$required" && ! -L "$bundle/$required" ]] || {
    echo "clean-home init bundle is missing $required" >&2
    exit 1
  }
done
[[ -d "$bundle/capsules" && ! -L "$bundle/capsules" ]] || {
  echo "clean-home init bundle is missing capsules directory" >&2
  exit 1
}

work=$(mktemp -d /tmp/aosinit.XXXXXX)
aos_home="$work/user/.aos"
project="$work/project"
mkdir -p "$project"

run_aos() {
  (
    cd "$project"
    HOME="$work/user" \
      AOS_HOME="$aos_home" \
      UNICITY_AOS_RUNTIME_BIN="$bundle/runtime/bin/astrid" \
      UNICITY_AOS_CAPSULE_DIR="$bundle/capsules" \
      "$bundle/bin/aos" "$@"
  )
}

snapshot_provenance() {
  ASTRID_HOME="$aos_home/runtime" ASTRID_RUN_DIR="$aos_home/run" \
    "$provenance_probe"
}

assert_exact_ready_capsules() {
  local ps_json
  ps_json=$(run_aos ps --format json)
  python3 - "$bundle/capsule-assets.txt" "$ps_json" <<'PY'
import json
import pathlib
import sys

assets_path = pathlib.Path(sys.argv[1])
rows = json.loads(sys.argv[2])
expected = sorted(
    line[:-len(".capsule")] if line.endswith(".capsule") else line
    for line in assets_path.read_text(encoding="utf-8").splitlines()
    if line
)
actual = sorted(row.get("capsule") for row in rows)
if actual != expected:
    raise SystemExit(
        f"running capsule set does not match the exact CE release set: "
        f"expected={expected!r}, actual={actual!r}"
    )
not_ready = [row for row in rows if row.get("state") != "ready"]
if not_ready:
    raise SystemExit(f"CE capsules are not all ready: {not_ready!r}")
PY
}

cleanup() {
  status=$?
  trap - EXIT
  run_aos stop >/dev/null 2>&1 || true
  if ! rm -rf "$work"; then
    echo "warning: clean-home fixture cleanup left runtime mounts for runner teardown" >&2
  fi
  exit "$status"
}
trap cleanup EXIT

run_aos init --offline --yes --var openai_api_key=release-gate-not-a-real-key

manifest="$aos_home/distributions/unicity-ce/Distro.toml"
[[ -f "$manifest" ]]
run_aos agent show default --format json > "$work/default.before.json"
snapshot_provenance > "$work/distro.before.json"

python3 - "$bundle/capsule-assets.txt" "$aos_home/runtime/etc/profiles/default.toml" "$work/distro.before.json" <<'PY'
import json
import pathlib
import sys
import tomllib

assets_path, profile_path, lock_path = map(pathlib.Path, sys.argv[1:])
expected = sorted(
    line[:-len(".capsule")] if line.endswith(".capsule") else line
    for line in assets_path.read_text(encoding="utf-8").splitlines()
    if line
)
if len(expected) != 22 or len(set(expected)) != 22:
    raise SystemExit("release capsule inventory is not the exact 22-capsule CE set")
profile = tomllib.loads(profile_path.read_text())
lock = json.loads(lock_path.read_text())
if lock.get("distro_id") != "unicity-ce":
    raise SystemExit("default principal has no CE distro provenance")
if sorted(item["name"] for item in lock["capsules"]) != expected:
    raise SystemExit("durable distro provenance does not bind the exact release capsule set")
granted = sorted(profile.get("capsules", []))
if granted != expected:
    raise SystemExit("default principal was not granted the exact release capsule set")
PY

assert_exact_ready_capsules
run_aos doctor

pid_file="$aos_home/run/system.pid"
[[ -f "$pid_file" && ! -L "$pid_file" ]]
cp "$pid_file" "$work/system.pid.before"
run_aos capsule show aos-cli --format json > "$work/cli-meta.before.json"
run_aos init --offline --yes --var openai_api_key=release-gate-not-a-real-key
snapshot_provenance > "$work/distro.after.json"
python3 - "$work/distro.before.json" "$work/distro.after.json" <<'PY'
import json
import pathlib
import sys
before, after = [json.loads(pathlib.Path(path).read_text()) for path in sys.argv[1:]]
for key in ("distro_id", "distro_version", "capsules", "manifest_hash"):
    if before.get(key) != after.get(key):
        raise SystemExit(f"reinitialization changed distro provenance: {key}")
PY
run_aos agent show default --format json > "$work/default.after.json"
run_aos capsule show aos-cli --format json > "$work/cli-meta.after.json"
cmp "$work/default.before.json" "$work/default.after.json"
cmp "$work/system.pid.before" "$pid_file"
cmp "$work/cli-meta.before.json" "$work/cli-meta.after.json"
assert_exact_ready_capsules

[[ -d "$project/.aos" ]]
if find "$work/user" "$project" -name .astrid -print -quit | grep -q .; then
  echo "clean AOS initialization created standalone Astrid state" >&2
  exit 1
fi

[[ -f "$pid_file" && ! -L "$pid_file" ]]
IFS= read -r daemon_pid < "$pid_file"
[[ "$daemon_pid" =~ ^[1-9][0-9]*$ ]]
kill -0 "$daemon_pid"

run_aos stop
for _ in $(seq 1 200); do
  if ! kill -0 "$daemon_pid" 2>/dev/null; then
    break
  fi
  sleep 0.05
done
if kill -0 "$daemon_pid" 2>/dev/null; then
  echo "AOS runtime process $daemon_pid remained alive after stop" >&2
  exit 1
fi
for transient in system.sock system.pid system.ready system.token; do
  [[ ! -e "$aos_home/run/$transient" && ! -L "$aos_home/run/$transient" ]]
done

python3 - "$aos_home/run/system.lock" "$aos_home/runtime" <<'PY'
import fcntl
import pathlib
import sys

lock_path = pathlib.Path(sys.argv[1])
if lock_path.is_symlink():
    raise SystemExit("stopped runtime lock is a symlink")
# Successful retirement may remove the runtime lock entirely. If retained,
# it must be unlocked; process exit and transient-marker checks precede this.
if lock_path.exists():
    with lock_path.open("r+b", buffering=0) as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        fcntl.flock(lock, fcntl.LOCK_UN)
runtime = pathlib.Path(sys.argv[2])
if sorted(path.name for path in runtime.iterdir()) != ["astrid.volume"]:
    raise SystemExit("stopped AOS runtime is not exactly astrid.volume")
PY

echo "clean AOS home initialized, loaded, rechecked, and stopped successfully"
