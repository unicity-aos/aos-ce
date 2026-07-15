#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <installed-aos-binary> <absolute-aos-home>" >&2
  exit 2
fi

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
aos_binary=$1
aos_home=$2
if [[ ! -x "$aos_binary" ]]; then
  echo "final runtime boot gate requires an installed candidate aos binary" >&2
  exit 1
fi
if [[ "$aos_home" != /* || ! -d "$aos_home/runtime" ]]; then
  echo "final runtime boot gate requires an absolute installed AOS home" >&2
  exit 1
fi

compatibility=$repo_root/release/runtime-compatibility.toml
if [[ -n "${AOS_FINAL_BOOT_TEST_COMPATIBILITY:-}" ]]; then
  if [[ "${AOS_FINAL_BOOT_TEST_MODE:-}" != 1 || "$AOS_FINAL_BOOT_TEST_COMPATIBILITY" != /* ]]; then
    echo "final runtime boot compatibility override is test-only and must be absolute" >&2
    exit 1
  fi
  compatibility=$AOS_FINAL_BOOT_TEST_COMPATIBILITY
fi
read -r runtime_version release_ready upgrade_ready < <(
  python3 - "$compatibility" <<'PY'
import sys
try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib

with open(sys.argv[1], "rb") as file:
    runtime = tomllib.load(file)["runtime"]
print(
    runtime["version"],
    str(runtime["release-ready"]).lower(),
    str(runtime["upgrade-self-heal-ready"]).lower(),
)
PY
)
if [[ "$release_ready" != true ]]; then
  echo "final runtime boot gate is pending: the exact approved runtime is not release-ready" >&2
  exit 1
fi
if [[ "$upgrade_ready" != true && "$upgrade_ready" != false ]]; then
  echo "final runtime boot gate could not read the upgrade-self-heal gate" >&2
  exit 1
fi

runtime_home=$aos_home/runtime
run_dir=$runtime_home/run
for stale in system.sock system.ready system.token deferred.db; do
  if [[ -e "$run_dir/$stale" || -L "$run_dir/$stale" ]]; then
    echo "final runtime boot gate requires clean regenerated coordination state: $run_dir/$stale exists" >&2
    exit 1
  fi
done

home=$(dirname "$aos_home")
run_aos() {
  HOME="$home" UNICITY_AOS_HOME="$aos_home" "$aos_binary" "$@"
}

assert_singleton_lock_available() {
  python3 - "$runtime_home/run/system.lock" <<'PY'
import fcntl
import os
import stat
import sys

path = sys.argv[1]
metadata = os.lstat(path)
if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
    raise SystemExit("stopped runtime singleton lock is not a real regular file")
with open(path, "r+b", buffering=0) as lock:
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError as error:
        raise SystemExit("stopped runtime singleton lock is still held") from error
    fcntl.flock(lock, fcntl.LOCK_UN)
PY
}

runtime_json=$(run_aos version --format json)
python3 - "$runtime_version" "$runtime_json" <<'PY'
import json
import sys

expected, encoded = sys.argv[1:]
actual = json.loads(encoded).get("version")
if actual != expected:
    raise SystemExit(f"candidate runtime version {actual!r} does not match exact pin {expected!r}")
PY

start_attempted=0
cleanup() {
  status=$?
  trap - EXIT
  if [[ "$start_attempted" -eq 1 ]] && ! run_aos stop >/dev/null 2>&1; then
    if [[ "$status" -eq 0 ]]; then
      status=1
    fi
  fi
  exit "$status"
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

start_attempted=1
run_aos start
if [[ ! -S "$run_dir/system.sock" || -L "$run_dir/system.sock" ]]; then
  echo "candidate runtime did not create a real system.sock" >&2
  exit 1
fi
if [[ ! -f "$run_dir/system.ready" || -L "$run_dir/system.ready" ]]; then
  echo "candidate runtime did not create a real system.ready" >&2
  exit 1
fi
if [[ ! -s "$run_dir/system.token" || -L "$run_dir/system.token" ]]; then
  echo "candidate runtime did not create a real non-empty system.token" >&2
  exit 1
fi
token_mode=$(stat -c '%a' "$run_dir/system.token" 2>/dev/null || stat -f '%Lp' "$run_dir/system.token")
if [[ "$token_mode" != 600 ]]; then
  echo "candidate runtime system.token mode is $token_mode, expected 600" >&2
  exit 1
fi
if [[ -e "$run_dir/deferred.db" || -L "$run_dir/deferred.db" ]]; then
  if [[ ! -d "$run_dir/deferred.db" || -L "$run_dir/deferred.db" ]]; then
    echo "candidate runtime deferred.db is not a real directory" >&2
    exit 1
  fi
  if [[ -e "$run_dir/deferred.db/stale-private-clone-sentinel" ]]; then
    echo "candidate runtime retained stale-private-clone-sentinel deferred state" >&2
    exit 1
  fi
fi

status_json=$(run_aos status --json)
python3 - "$runtime_version" "$status_json" <<'PY'
import json
import sys

expected, encoded = sys.argv[1:]
status = json.loads(encoded)
if status.get("state") != "running":
    raise SystemExit(f"candidate runtime is not running: {status!r}")
if status.get("runtime_version") != expected:
    raise SystemExit(
        f"running runtime version {status.get('runtime_version')!r} does not match {expected!r}"
    )
PY

run_aos stop
for cleaned in system.sock system.ready system.token; do
  [[ ! -e "$run_dir/$cleaned" && ! -L "$run_dir/$cleaned" ]]
done
stopped_status_json=$(run_aos status --json)
python3 - "$runtime_version" "$stopped_status_json" <<'PY'
import json
import sys

expected, encoded = sys.argv[1:]
status = json.loads(encoded)
if status.get("state") != "stopped":
    raise SystemExit(f"candidate runtime did not report typed stopped state: {status!r}")
if status.get("runtime_version") != expected:
    raise SystemExit(
        f"stopped runtime version {status.get('runtime_version')!r} does not match {expected!r}"
    )
PY
assert_singleton_lock_available
start_attempted=0

echo "exact candidate runtime boot and regenerated coordination checks passed"
