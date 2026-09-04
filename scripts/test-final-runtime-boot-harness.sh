#!/usr/bin/env bash
set -euo pipefail
trap 'echo "final runtime boot harness failed at line $LINENO" >&2' ERR

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
work=$(mktemp -d /tmp/aosboot.XXXXXX)
trap 'pkill -F "$work/current/state/daemon.pid" 2>/dev/null || true; rm -rf "$work"' EXIT

cat > "$work/fake-aos.py" <<'PY'
#!/usr/bin/env python3
import fcntl
import json
import os
import signal
import socket
import subprocess
import sys
import time
from pathlib import Path

state_dir = Path(os.environ["FAKE_STATE_DIR"])
aos_home = Path(os.environ["AOS_HOME"])
run_dir_value = os.environ.get("ASTRID_RUN_DIR")
if not run_dir_value:
    raise SystemExit("fake runtime requires ASTRID_RUN_DIR")
run_dir = Path(run_dir_value)
if not run_dir.is_absolute() or run_dir.is_symlink() or not run_dir.is_dir():
    raise SystemExit("fake runtime requires ASTRID_RUN_DIR to be an absolute real directory")
state_dir.mkdir(parents=True, exist_ok=True)
command = sys.argv[1] if len(sys.argv) > 1 else ""


def state() -> str:
    path = state_dir / "state"
    return path.read_text(encoding="utf-8").strip() if path.exists() else "stopped"


def remove_coordination() -> None:
    for name in ("system.sock", "system.ready", "system.token"):
        path = run_dir / name
        if path.exists() or path.is_symlink():
            path.unlink()


if command == "_daemon":
    lock_path = run_dir / "system.lock"
    with lock_path.open("r+b", buffering=0) as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        server = socket.socket(socket.AF_UNIX)
        server.bind(str(run_dir / "system.sock"))
        server.listen(1)
        (run_dir / "system.token").write_text("fresh-token\n", encoding="utf-8")
        os.chmod(run_dir / "system.token", int(os.environ.get("FAKE_TOKEN_MODE", "0600"), 8))
        (run_dir / "deferred.db").mkdir(exist_ok=True)
        if os.environ.get("FAKE_STALE_DEFERRED") == "1":
            (run_dir / "deferred.db/stale-private-clone-sentinel").write_text(
                "stale\n", encoding="utf-8"
            )
        (run_dir / "system.ready").write_text("ready\n", encoding="utf-8")
        (state_dir / "state").write_text("running\n", encoding="utf-8")
        running = True

        def stop(_signal: int, _frame: object) -> None:
            global running
            running = False

        signal.signal(signal.SIGTERM, stop)
        signal.signal(signal.SIGINT, stop)
        while running:
            time.sleep(0.02)
        remove_coordination()
        (state_dir / "state").write_text("stopped\n", encoding="utf-8")
        server.close()
    raise SystemExit(0)

if command == "version":
    print(json.dumps({"version": os.environ.get("FAKE_VERSION", "9.9.9")}))
    raise SystemExit(0)

if command == "start":
    process = subprocess.Popen(
        [sys.executable, __file__, "_daemon"],
        env=os.environ.copy(),
        start_new_session=True,
    )
    (state_dir / "daemon.pid").write_text(f"{process.pid}\n", encoding="utf-8")
    for _ in range(500):
        if (run_dir / "system.ready").exists():
            break
        if process.poll() is not None:
            raise SystemExit(8)
        time.sleep(0.01)
    else:
        raise SystemExit(9)
    raise SystemExit(7 if os.environ.get("FAKE_PARTIAL_START") == "1" else 0)

if command == "stop":
    count = state_dir / "stop-count"
    previous = int(count.read_text(encoding="utf-8")) if count.exists() else 0
    count.write_text(f"{previous + 1}\n", encoding="utf-8")
    pid_path = state_dir / "daemon.pid"
    if pid_path.exists():
        pid = int(pid_path.read_text(encoding="utf-8"))
        if os.environ.get("FAKE_LEAVE_LOCK") == "1":
            remove_coordination()
            (state_dir / "state").write_text("stopped\n", encoding="utf-8")
            raise SystemExit(0)
        try:
            os.kill(pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        for _ in range(500):
            try:
                os.kill(pid, 0)
            except ProcessLookupError:
                break
            time.sleep(0.01)
    raise SystemExit(0)

if command == "status":
    current = state()
    (state_dir / "status-entered").write_text(f"{current}\n", encoding="utf-8")
    if current == "running":
        time.sleep(float(os.environ.get("FAKE_STATUS_DELAY", "0")))
        current = os.environ.get("FAKE_RUNNING_STATUS", current)
    else:
        current = os.environ.get("FAKE_STOPPED_STATUS", current)
    print(json.dumps({"state": current, "runtime_version": os.environ.get("FAKE_VERSION", "9.9.9")}))
    raise SystemExit(0)

raise SystemExit(2)
PY
chmod 755 "$work/fake-aos.py"

cat > "$work/compatibility.toml" <<'EOF'
schema-version = 1
[runtime]
version = "9.9.9"
release-ready = true
upgrade-self-heal-ready = false
EOF

cat > "$work/pending-compatibility.toml" <<'EOF'
schema-version = 1
[runtime]
version = "9.9.9"
release-ready = false
upgrade-self-heal-ready = false
EOF

new_case() {
  rm -rf "$work/current"
  mkdir -p "$work/current/home/.aos/runtime" "$work/current/home/.aos/run" "$work/current/state"
  : > "$work/current/home/.aos/run/system.lock"
  chmod 600 "$work/current/home/.aos/run/system.lock"
}

run_gate() {
  run_gate_env
}

# Arguments are environment assignments supplied by individual negative cases.
# shellcheck disable=SC2120
run_gate_env() {
  env "$@" \
    AOS_FINAL_BOOT_TEST_MODE=1 \
    AOS_FINAL_BOOT_TEST_COMPATIBILITY="$work/compatibility.toml" \
    FAKE_STATE_DIR="$work/current/state" \
    "$repo_root/scripts/test-final-runtime-boot.sh" \
    "$work/fake-aos.py" \
    "$work/current/home/.aos"
}

expect_failure() {
  local pattern=$1
  shift
  if "$@" > "$work/failure.log" 2>&1; then
    echo "final runtime boot gate unexpectedly succeeded" >&2
    exit 1
  fi
  if [[ "$pattern" != - ]]; then
    grep -E "$pattern" "$work/failure.log" >/dev/null
  fi
}

new_case
expect_failure 'not release-ready' env \
  AOS_FINAL_BOOT_TEST_MODE=1 \
  AOS_FINAL_BOOT_TEST_COMPATIBILITY="$work/pending-compatibility.toml" \
  FAKE_STATE_DIR="$work/current/state" \
  "$repo_root/scripts/test-final-runtime-boot.sh" \
  "$work/fake-aos.py" \
  "$work/current/home/.aos"

cleanup_daemon() {
  local pid_file=$work/current/state/daemon.pid
  if [[ -f "$pid_file" ]]; then
    kill -TERM "$(cat "$pid_file")" 2>/dev/null || true
    for _ in $(seq 1 200); do
      kill -0 "$(cat "$pid_file")" 2>/dev/null || break
      sleep 0.01
    done
  fi
}

new_case
run_gate >/dev/null
test "$(cat "$work/current/state/state")" = stopped
test "$(cat "$work/current/state/stop-count")" -eq 1

# Keep the gate paused in its running window so the path contract is checked
# independently of the post-stop cleanup assertions in the gate itself.
new_case
run_gate_env FAKE_STATUS_DELAY=2 >"$work/success.log" 2>&1 &
success_pid=$!
success_state=$work/current/state
success_home=$work/current/home/.aos
for _ in $(seq 1 500); do
  if [[ -f "$success_state/status-entered" ]]; then
    break
  fi
  if ! kill -0 "$success_pid" 2>/dev/null; then
    wait "$success_pid"
    echo "final runtime boot gate exited before its running window" >&2
    exit 1
  fi
  sleep 0.01
done
test -f "$success_state/status-entered"
test "$(cat "$success_state/status-entered")" = running
test -S "$success_home/run/system.sock"
test -f "$success_home/run/system.ready"
test -s "$success_home/run/system.token"
for coordination in system.sock system.ready system.token; do
  [[ ! -e "$success_home/runtime/run/$coordination" && ! -L "$success_home/runtime/run/$coordination" ]]
done
wait "$success_pid"
for coordination in system.sock system.ready system.token; do
  [[ ! -e "$success_home/run/$coordination" && ! -L "$success_home/run/$coordination" ]]
  [[ ! -e "$success_home/runtime/run/$coordination" && ! -L "$success_home/runtime/run/$coordination" ]]
done

new_case
expect_failure 'does not match exact pin' run_gate_env FAKE_VERSION=9.9.8

for stale in system.sock system.ready system.token deferred.db; do
  new_case
  case "$stale" in
    deferred.db) mkdir "$work/current/home/.aos/run/$stale" ;;
    *) : > "$work/current/home/.aos/run/$stale" ;;
  esac
  expect_failure 'requires clean regenerated coordination state' run_gate
done

for linked in system.sock system.ready system.token; do
  new_case
  ln -s "$work/current/outside" "$work/current/home/.aos/run/$linked"
  expect_failure 'requires clean regenerated coordination state' run_gate
done

new_case
expect_failure 'system.token' run_gate_env FAKE_TOKEN_MODE=0644
test "$(cat "$work/current/state/state")" = stopped

new_case
expect_failure 'stale-private-clone-sentinel' run_gate_env FAKE_STALE_DEFERRED=1
test "$(cat "$work/current/state/state")" = stopped

new_case
expect_failure 'not running' run_gate_env FAKE_RUNNING_STATUS=stopped
test "$(cat "$work/current/state/state")" = stopped

new_case
expect_failure 'typed stopped state' run_gate_env FAKE_STOPPED_STATUS=running

new_case
expect_failure 'singleton lock is still held' run_gate_env FAKE_LEAVE_LOCK=1
cleanup_daemon

new_case
expect_failure - run_gate_env FAKE_PARTIAL_START=1
test "$(cat "$work/current/state/state")" = stopped
test "$(cat "$work/current/state/stop-count")" -ge 1

for signal in HUP INT TERM; do
  new_case
  python3 - \
    "$signal" \
    "$repo_root/scripts/test-final-runtime-boot.sh" \
    "$work/fake-aos.py" \
    "$work/current/home/.aos" \
    "$work/current/state" \
    "$work/compatibility.toml" \
    "$work/signal.log" <<'PY'
import os
import signal
import subprocess
import sys
import time

name, hook, candidate, home, state, compatibility, log = sys.argv[1:]
expected = {"HUP": 129, "INT": 130, "TERM": 143}[name]
environment = os.environ.copy()
environment.update(
    {
        "AOS_FINAL_BOOT_TEST_MODE": "1",
        "AOS_FINAL_BOOT_TEST_COMPATIBILITY": compatibility,
        "FAKE_STATE_DIR": state,
        "FAKE_STATUS_DELAY": "2",
    }
)
with open(log, "wb") as output:
    process = subprocess.Popen(
        [hook, candidate, home],
        env=environment,
        stdout=output,
        stderr=subprocess.STDOUT,
    )
    marker = os.path.join(state, "status-entered")
    for _ in range(500):
        if os.path.isfile(marker):
            break
        if process.poll() is not None:
            raise SystemExit(f"boot hook exited before {name} could be delivered")
        time.sleep(0.01)
    else:
        process.kill()
        raise SystemExit(f"boot hook did not reach running status before {name}")
    os.kill(process.pid, getattr(signal, f"SIG{name}"))
    status = process.wait(timeout=10)
if status != expected:
    raise SystemExit(f"boot hook exited {status} for {name}, expected {expected}")
PY
  test "$(cat "$work/current/state/state")" = stopped
  test "$(cat "$work/current/state/stop-count")" -ge 1
done

echo "final runtime boot gate harness passed"
