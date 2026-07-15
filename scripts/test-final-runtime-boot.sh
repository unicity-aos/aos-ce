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

read -r runtime_version release_ready upgrade_ready < <(
  python3 - "$repo_root/release/runtime-compatibility.toml" <<'PY'
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

runtime_json=$(run_aos version --format json)
python3 - "$runtime_version" "$runtime_json" <<'PY'
import json
import sys

expected, encoded = sys.argv[1:]
actual = json.loads(encoded).get("version")
if actual != expected:
    raise SystemExit(f"candidate runtime version {actual!r} does not match exact pin {expected!r}")
PY

started=0
cleanup() {
  status=$?
  trap - EXIT HUP INT TERM
  if [[ "$started" -eq 1 ]]; then
    run_aos stop >/dev/null 2>&1 || status=1
  fi
  exit "$status"
}
trap cleanup EXIT HUP INT TERM

run_aos start
started=1
[[ -S "$run_dir/system.sock" ]]
[[ -f "$run_dir/system.ready" && ! -L "$run_dir/system.ready" ]]
[[ -s "$run_dir/system.token" && ! -L "$run_dir/system.token" ]]
token_mode=$(stat -c '%a' "$run_dir/system.token" 2>/dev/null || stat -f '%Lp' "$run_dir/system.token")
[[ "$token_mode" == 600 ]]
if [[ -e "$run_dir/deferred.db" || -L "$run_dir/deferred.db" ]]; then
  [[ -d "$run_dir/deferred.db" && ! -L "$run_dir/deferred.db" ]]
  [[ ! -e "$run_dir/deferred.db/stale-private-clone-sentinel" ]]
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
started=0
for cleaned in system.sock system.ready system.token; do
  [[ ! -e "$run_dir/$cleaned" && ! -L "$run_dir/$cleaned" ]]
done

echo "exact candidate runtime boot and regenerated coordination checks passed"
