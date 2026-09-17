#!/usr/bin/env bash
# Full packaged-guest journey in an isolated home. Never selects the live AOS.
set -euo pipefail
umask 077
if [[ $# -lt 3 || $# -gt 4 || "$1" != /* || "$2" != /* || "$3" != /* ]]; then
  echo 'usage: bash test-native-guest.sh /absolute/astrid /absolute/native_input_guest_setup /absolute/probe.capsule [/absolute/released-astrid]' >&2
  exit 2
fi
cli=$1
setup=$2
capsule=$3
baseline=${4:-$cli}
[[ -x "$cli" && -x "$setup" && -f "$capsule" ]] || exit 2
[[ "$baseline" == /* && -x "$baseline" ]] || exit 2
package_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
test_root=$(mktemp -d /private/tmp/ani-guest.XXXXXX)
mkdir -m 700 "$test_root/runtime"
export ASTRID_HOME="$test_root/runtime"
export ASTRID_PRINCIPAL=default
unset ASTRID_RUN_DIR ASTRID_ENFORCED_DISTRO ASTRID_CLIENT_CONFIG_PATH
cd "$test_root"
cleanup() {
  local result=$?
  trap - EXIT
  cd "$test_root"
  if ! "$cli" stop > "$test_root/cleanup.log" 2>&1; then
    echo "Disposable runtime cleanup failed; inspect $test_root/cleanup.log" >&2
    result=1
  fi
  printf 'Retained disposable evidence: %s\n' "$test_root"
  exit "$result"
}
trap cleanup EXIT
# JSON strings are also valid TOML basic strings. Escape the caller's path.
capsule_literal=$(python3 -c 'import json,sys; print(json.dumps(sys.argv[1]))' "$capsule")
printf 'schema-version = 1\n[distro]\nid = "native-input-test"\nname = "Native input test"\nversion = "0.1.0"\n[[capsule]]\nname = "native-input-probe"\nversion = "0.1.0"\nrole = "uplink"\nsource = %s\n' "$capsule_literal" > Distro.toml
"$baseline" init --distro "$test_root/Distro.toml" --offline --allow-unsigned --yes --grant-capsules
if [[ "$baseline" != "$cli" ]]; then
  # Exercise real principal provisioning in the released binary. This is not
  # an Oracle install: it separately checks that upgrade preserves these keys.
  for principal in codex-code claude-code grok-code; do
    "$baseline" agent create "$principal"
  done
  for principal in default codex-code claude-code grok-code; do
    shasum -a 256 "$ASTRID_HOME/keys/$principal.key"
  done > "$test_root/principal-keys.before.sha256"
  # Real released daemon creates and retires this home before the candidate
  # opens the very same volume. No reconstructed migration fixture or reinstall.
  "$baseline" stop
  python3 -c 'import os,sys; assert os.listdir(sys.argv[1]) == ["astrid.volume"], "baseline did not retire to volume-only"' "$ASTRID_HOME"
  shasum -a 256 "$baseline" "$cli" "$capsule" > "$test_root/upgrade-inputs.sha256"
  "$cli" start
  for principal in default codex-code claude-code grok-code; do
    shasum -a 256 "$ASTRID_HOME/keys/$principal.key"
  done > "$test_root/principal-keys.after.sha256"
  cmp "$test_root/principal-keys.before.sha256" "$test_root/principal-keys.after.sha256"
fi
# Enroll through the real public API. The helper below only selects the
# already-paired responder in operator config; it never edits auth profiles.
"$cli" keypair generate --name native-ui --raw > "$test_root/device-public-key.txt"
public_key=$(<"$test_root/device-public-key.txt")
"$cli" pair-device issue --scope use-only --label native-ui --raw > "$test_root/device-pair-token.txt"
"$cli" pair-device redeem --public-key "$public_key" < "$test_root/device-pair-token.txt" > "$test_root/device-paired.json"
"$setup" "$test_root"
"$cli" stop
"$cli" start
cd "$package_dir"
ASTRID_NATIVE_GUEST_ROOT="$test_root" ASTRID_NATIVE_GUEST_CLI="$cli" \
  swift test -Xswiftc -warnings-as-errors --filter NativeGuestJourneyTests
