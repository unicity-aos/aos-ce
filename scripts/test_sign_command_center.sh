#!/usr/bin/env bash
# Fixture and workflow-contract checks for Command Center Developer ID signing.
# Does not call Apple, export keys, or notarize a real app.
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
sign="$repo_root/scripts/sign-command-center.sh"
workflow="$repo_root/.github/workflows/release.yml"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

fail() {
  echo "$1" >&2
  exit 1
}

expect_fail() {
  local log=$1
  local needle=$2
  shift 2
  if "$@" >"$log.out" 2>"$log.err"; then
    fail "expected failure: $needle"
  fi
  grep -Fq "$needle" "$log.err" || {
    echo "missing error text: $needle" >&2
    cat "$log.err" >&2
    exit 1
  }
}

make_app() {
  local app=$1
  mkdir -p "$app/Contents/MacOS"
  printf 'fixture\n' > "$app/Contents/MacOS/aos-tray"
  chmod 755 "$app/Contents/MacOS/aos-tray"
}

install_tools() {
  local dir=$1
  local dump=$2
  mkdir -p "$dir"
  cat >"$dir/codesign" <<'SH'
#!/bin/sh
set -eu
dump=${FAKE_CODESIGN_DUMP:?}
log=${FAKE_CODESIGN_LOG:?}
imported=${FAKE_SECURITY_IMPORTED_FLAG:?}
keychain_file=${FAKE_SECURITY_KEYCHAIN_FILE:?}
printf '%s\n' "$*" >> "$log"
if [ "$1" = "--force" ]; then
  if [ ! -f "$imported" ]; then
    echo "codesign ran before PKCS12 import" >&2
    exit 1
  fi
  expected=$(cat "$keychain_file")
  saw_keychain=0
  prev=
  for arg in "$@"; do
    if [ "$prev" = "--keychain" ]; then
      if [ "$arg" != "$expected" ]; then
        echo "codesign --keychain did not use the created keychain" >&2
        exit 1
      fi
      saw_keychain=1
    fi
    prev=$arg
  done
  if [ "$saw_keychain" != 1 ]; then
    echo "codesign --force missing --keychain" >&2
    exit 1
  fi
  exit 0
fi
if [ "$1" = "--verify" ]; then
  exit 0
fi
if [ "$1" = "-d" ]; then
  cat "$dump"
  exit 0
fi
echo "unexpected codesign invocation: $*" >&2
exit 1
SH
  cat >"$dir/security" <<'SH'
#!/bin/sh
set -eu
log=${FAKE_SECURITY_LOG:?}
keychain_file=${FAKE_SECURITY_KEYCHAIN_FILE:?}
imported=${FAKE_SECURITY_IMPORTED_FLAG:?}
printf '%s\n' "$*" >> "$log"
cmd=$1
shift
keychain=
prev=
p12=
codesign_allow=
import_keychain=
for arg in "$@"; do
  if [ "$prev" = "-T" ]; then
    codesign_allow=$arg
  fi
  if [ "$cmd" = import ] && [ "$prev" = "-k" ]; then
    import_keychain=$arg
  fi
  prev=$arg
  case "$arg" in
    *.p12|*.P12) p12=$arg ;;
  esac
done
eval "keychain=\${$#}"
if [ "$cmd" = import ]; then
  keychain=$import_keychain
fi
case "$cmd" in
  create-keychain)
    [ -n "$keychain" ]
    mkdir -p "$(dirname "$keychain")"
    printf 'keychain\n' > "$keychain"
    printf '%s\n' "$keychain" > "$keychain_file"
    exit 0
    ;;
  set-keychain-settings|unlock-keychain|set-key-partition-list)
    [ -n "$keychain" ]
    [ -f "$keychain" ]
    exit 0
    ;;
  import)
    [ -n "$keychain" ]
    [ -f "$keychain" ]
    [ -n "$p12" ] && [ -f "$p12" ] && [ ! -L "$p12" ]
    [ -n "$codesign_allow" ]
    printf 'imported\n' > "$imported"
    exit 0
    ;;
  delete-keychain)
    [ -n "$keychain" ]
    rm -f "$keychain"
    exit 0
    ;;
  list-keychains|default-keychain)
    echo "security $cmd must not mutate the runner keychain search list" >&2
    exit 1
    ;;
  find-identity|export)
    echo "security $cmd is forbidden in Command Center signing" >&2
    exit 1
    ;;
  *)
    echo "unexpected security invocation: $cmd $*" >&2
    exit 1
    ;;
esac
SH
  cat >"$dir/ditto" <<'SH'
#!/bin/sh
set -eu
zip=
for arg in "$@"; do
  zip=$arg
done
[ -n "$zip" ]
mkdir -p "$(dirname "$zip")"
printf 'zip\n' > "$zip"
exit 0
SH
  cat >"$dir/xcrun" <<'SH'
#!/bin/sh
set -eu
[ "$1" = notarytool ]
[ "$2" = submit ]
zip=$3
[ -f "$zip" ]
saw_wait=0
for arg in "$@"; do
  [ "$arg" = "--wait" ] && saw_wait=1
done
[ "$saw_wait" = 1 ]
if [ "${FAKE_NOTARY_MODE:-key}" = apple ]; then
  shift 3
  [ "$#" = 7 ]
  [ "$1" = --apple-id ] && [ "$2" = fixture@example.invalid ]
  [ "$3" = --team-id ] && [ "$4" = TEAMID12AB ]
  [ "$5" = --password ] && [ "$6" = fixture-app-password ]
  [ "$7" = --wait ]
fi
if [ "${FAKE_NOTARY_FAIL:-0}" = 1 ]; then
  echo 'fixture notarization rejected' >&2
  exit 1
fi
printf 'accepted\n' >&2
exit 0
SH
  cat >"$dir/stapler" <<'SH'
#!/bin/sh
set -eu
[ "$1" = staple ] || [ "$1" = validate ]
[ -z "${FAKE_STAPLER_LOG:-}" ] || printf '%s\n' "$1" >> "$FAKE_STAPLER_LOG"
exit 0
SH
  chmod 755 "$dir/codesign" "$dir/security" "$dir/ditto" "$dir/xcrun" "$dir/stapler"
}

write_dump() {
  local dump=$1
  local team=$2
  local identifier=$3
  cat >"$dump" <<DUMP
Executable=/fixture/AOS Command Center.app/Contents/MacOS/aos-tray
Identifier=$identifier
Format=app bundle with Mach-O thin (arm64)
CodeDirectory v=20400 size=123 flags=0x0(none) hashes=10+3 location=embedded
Signature size=1234
Authority=Developer ID Application: Unicity Labs ($team)
Authority=Developer ID Certification Authority
Authority=Apple Root CA
Timestamp=1 Jan 2026 at 00:00:00
Info.plist entries=12
TeamIdentifier=$team
Runtime Version=14.0.0
Sealed Resources version=2 rules=4 files=2
Internal requirements count=1 size=180
DUMP
}

aos_env() {
  export AOS_MACOS_DEVELOPMENT_TEAM_ID=TEAMID12AB
  export AOS_MACOS_DEVELOPER_ID_IDENTITY="Developer ID Application: Unicity Labs (TEAMID12AB)"
  export AOS_MACOS_NOTARY_KEY_ID=key-id
  export AOS_MACOS_NOTARY_ISSUER_ID=issuer-id
  export AOS_MACOS_CERTIFICATE_PASSWORD=p12-pass
}

write_p12() {
  local path=$1
  printf 'fixture-pkcs12\n' > "$path"
  chmod 600 "$path"
  export AOS_MACOS_CERTIFICATE_P12_PATH="$path"
}

# Missing AOS credentials fail closed before any tool runs.
unset AOS_MACOS_DEVELOPMENT_TEAM_ID AOS_MACOS_DEVELOPER_ID_IDENTITY \
  AOS_MACOS_NOTARY_KEY_PATH AOS_MACOS_NOTARY_KEY_ID AOS_MACOS_NOTARY_ISSUER_ID \
  AOS_MACOS_NOTARY_PROFILE AOS_MACOS_TOOL_DIR \
  AOS_MACOS_NOTARY_APPLE_ID AOS_MACOS_NOTARY_APP_PASSWORD \
  AOS_MACOS_CERTIFICATE_P12_PATH AOS_MACOS_CERTIFICATE_PASSWORD || true
make_app "$work/missing.app"
expect_fail "$work/missing-env" "AOS_MACOS_DEVELOPMENT_TEAM_ID is required" \
  env -u AOS_MACOS_DEVELOPMENT_TEAM_ID -u AOS_MACOS_DEVELOPER_ID_IDENTITY \
  sh "$sign" --app "$work/missing.app" --output "$work/missing-out"

# Astrid-only credentials are refused rather than borrowed.
make_app "$work/astrid.app"
expect_fail "$work/astrid-only" "Command Center signing does not borrow Astrid credentials" \
  env -u AOS_MACOS_DEVELOPMENT_TEAM_ID -u AOS_MACOS_DEVELOPER_ID_IDENTITY \
    ASTRID_MACOS_DEVELOPMENT_TEAM_ID=ASTRIDTEAM \
    ASTRID_MACOS_DEVELOPER_ID_IDENTITY="Developer ID Application" \
  sh "$sign" --app "$work/astrid.app" --output "$work/astrid-out"

# Ad-hoc identities are refused.
make_app "$work/adhoc.app"
aos_env
expect_fail "$work/adhoc-identity" "Command Center signing refuses an ad-hoc identity" \
  env AOS_MACOS_DEVELOPER_ID_IDENTITY="-" \
  sh "$sign" --app "$work/adhoc.app" --output "$work/adhoc-out"
expect_fail "$work/adhoc-name" "Command Center signing refuses an ad-hoc identity" \
  env AOS_MACOS_DEVELOPER_ID_IDENTITY="ad-hoc" \
  sh "$sign" --app "$work/adhoc.app" --output "$work/adhoc-name-out"

# PKCS12 path and password fail closed before tools.
make_app "$work/missing-p12.app"
aos_env
expect_fail "$work/missing-p12" "AOS_MACOS_CERTIFICATE_P12_PATH is required" \
  env -u AOS_MACOS_CERTIFICATE_P12_PATH \
  sh "$sign" --app "$work/missing-p12.app" --output "$work/missing-p12-out"
write_p12 "$work/fixture.p12"
expect_fail "$work/missing-p12-password" "AOS_MACOS_CERTIFICATE_PASSWORD is required" \
  env -u AOS_MACOS_CERTIFICATE_PASSWORD \
  sh "$sign" --app "$work/missing-p12.app" --output "$work/missing-p12-password-out"
ln -s "$work/fixture.p12" "$work/fixture.p12.link"
expect_fail "$work/p12-symlink" "AOS_MACOS_CERTIFICATE_P12_PATH must be a regular file" \
  env AOS_MACOS_CERTIFICATE_P12_PATH="$work/fixture.p12.link" \
  sh "$sign" --app "$work/missing-p12.app" --output "$work/p12-symlink-out"

# Tool wrappers live in AOS_MACOS_TOOL_DIR; PATH must not be used.
tool_dir="$work/tools"
path_trap="$work/path-trap"
mkdir -p "$path_trap"
cat >"$path_trap/codesign" <<'SH'
#!/bin/sh
echo "used PATH codesign" >&2
exit 1
SH
cat >"$path_trap/security" <<'SH'
#!/bin/sh
echo "used PATH security" >&2
exit 1
SH
chmod 755 "$path_trap/codesign" "$path_trap/security"
export PATH="$path_trap:$PATH"
export AOS_MACOS_TOOL_DIR="$tool_dir"
aos_env
write_p12 "$work/fixture.p12"
export FAKE_SECURITY_LOG="$work/security.log"
export FAKE_CODESIGN_LOG="$work/codesign.log"
export FAKE_SECURITY_KEYCHAIN_FILE="$work/created-keychain-path"
export FAKE_SECURITY_IMPORTED_FLAG="$work/imported-flag"
: > "$FAKE_SECURITY_LOG"
: > "$FAKE_CODESIGN_LOG"
rm -f "$FAKE_SECURITY_KEYCHAIN_FILE" "$FAKE_SECURITY_IMPORTED_FLAG"

write_dump "$work/team.dump" "OTHERTEAM1" "ai.unicity.aos.tray"
install_tools "$tool_dir" "$work/team.dump"
make_app "$work/team.app"
expect_fail "$work/team-mismatch" \
  "Command Center TeamIdentifier must match AOS_MACOS_DEVELOPMENT_TEAM_ID" \
  env FAKE_CODESIGN_DUMP="$work/team.dump" \
  sh "$sign" --app "$work/team.app" --output "$work/team-out"

# Exact-line identity: a longer TeamIdentifier must not satisfy TEAMID12AB.
write_dump "$work/team-spoof.dump" "TEAMID12ABEXTRA" "ai.unicity.aos.tray"
install_tools "$tool_dir" "$work/team-spoof.dump"
make_app "$work/team-spoof.app"
: > "$FAKE_SECURITY_LOG"
: > "$FAKE_CODESIGN_LOG"
rm -f "$FAKE_SECURITY_KEYCHAIN_FILE" "$FAKE_SECURITY_IMPORTED_FLAG"
expect_fail "$work/team-spoof" \
  "Command Center TeamIdentifier must match AOS_MACOS_DEVELOPMENT_TEAM_ID" \
  env FAKE_CODESIGN_DUMP="$work/team-spoof.dump" \
  sh "$sign" --app "$work/team-spoof.app" --output "$work/team-spoof-out"

write_dump "$work/id.dump" "TEAMID12AB" "ai.unicity.aos.wrong"
install_tools "$tool_dir" "$work/id.dump"
make_app "$work/id.app"
: > "$FAKE_SECURITY_LOG"
: > "$FAKE_CODESIGN_LOG"
rm -f "$FAKE_SECURITY_KEYCHAIN_FILE" "$FAKE_SECURITY_IMPORTED_FLAG"
expect_fail "$work/id-mismatch" \
  "Command Center identifier must remain ai.unicity.aos.tray" \
  env FAKE_CODESIGN_DUMP="$work/id.dump" \
  sh "$sign" --app "$work/id.app" --output "$work/id-out"

# Exact-line identity: a longer Identifier must not satisfy ai.unicity.aos.tray.
write_dump "$work/id-spoof.dump" "TEAMID12AB" "ai.unicity.aos.tray.spoof"
install_tools "$tool_dir" "$work/id-spoof.dump"
make_app "$work/id-spoof.app"
: > "$FAKE_SECURITY_LOG"
: > "$FAKE_CODESIGN_LOG"
rm -f "$FAKE_SECURITY_KEYCHAIN_FILE" "$FAKE_SECURITY_IMPORTED_FLAG"
expect_fail "$work/id-spoof" \
  "Command Center identifier must remain ai.unicity.aos.tray" \
  env FAKE_CODESIGN_DUMP="$work/id-spoof.dump" \
  sh "$sign" --app "$work/id-spoof.app" --output "$work/id-spoof-out"

# Success fixture: import into ephemeral keychain, bind codesign --keychain, cleanup.
write_dump "$work/ok.dump" "TEAMID12AB" "ai.unicity.aos.tray"
install_tools "$tool_dir" "$work/ok.dump"
make_app "$work/ok.app"
printf 'not-a-secret\n' > "$work/notary.p8"
chmod 600 "$work/notary.p8"
: > "$FAKE_SECURITY_LOG"
: > "$FAKE_CODESIGN_LOG"
rm -f "$FAKE_SECURITY_KEYCHAIN_FILE" "$FAKE_SECURITY_IMPORTED_FLAG"
signed=$(
  FAKE_CODESIGN_DUMP="$work/ok.dump" \
  AOS_MACOS_NOTARY_KEY_PATH="$work/notary.p8" \
  sh "$sign" --app "$work/ok.app" --output "$work/ok-out" 2>"$work/ok-sign.err"
)
[ "$signed" = "$work/ok-out/AOS Command Center.app" ] || fail "success path did not print the signed app"
[ -d "$signed" ] || fail "signed app missing"
[ ! -L "$signed" ] || fail "signed app became a symlink"
[ -f "$signed/Contents/MacOS/aos-tray" ] || fail "signed app lost its executable"
[ -f "$FAKE_SECURITY_KEYCHAIN_FILE" ] || fail "security create-keychain did not record a keychain"
created_keychain=$(cat "$FAKE_SECURITY_KEYCHAIN_FILE")
[ -n "$created_keychain" ] || fail "created keychain path was empty"
[ ! -e "$created_keychain" ] || fail "ephemeral keychain survived cleanup"
grep -q '^create-keychain ' "$FAKE_SECURITY_LOG" || fail "security create-keychain was not invoked"
grep -q '^set-keychain-settings ' "$FAKE_SECURITY_LOG" || fail "security set-keychain-settings was not invoked"
grep -q '^unlock-keychain ' "$FAKE_SECURITY_LOG" || fail "security unlock-keychain was not invoked"
grep -q '^import ' "$FAKE_SECURITY_LOG" || fail "security import was not invoked"
grep -q '^set-key-partition-list ' "$FAKE_SECURITY_LOG" || fail "security set-key-partition-list was not invoked"
grep -q '^delete-keychain ' "$FAKE_SECURITY_LOG" || fail "security delete-keychain was not invoked"
if grep -Eq '^(list-keychains|default-keychain)( |$)' "$FAKE_SECURITY_LOG"; then
  fail "helper mutated the runner keychain search list"
fi
grep -Fq -- "-k $created_keychain" "$FAKE_SECURITY_LOG" \
  || fail "import did not target the created keychain"
grep -Fq -- "-T $tool_dir/codesign" "$FAKE_SECURITY_LOG" || fail "import did not allow the codesign we invoke"
grep -Fq -- "--keychain $created_keychain" "$FAKE_CODESIGN_LOG" || fail "codesign did not bind --keychain to the created keychain"
if grep '^create-keychain ' "$FAKE_SECURITY_LOG" | grep -Fq 'p12-pass'; then
  fail "ephemeral keychain password reused the PKCS12 password"
fi
if grep -Eq '^(find-identity|export)( |$)' "$FAKE_SECURITY_LOG"; then
  fail "helper consulted or exported key material"
fi

# Apple-ID notarization uses the signing team and preserves rejection behavior.
export FAKE_NOTARY_MODE=apple
export AOS_MACOS_NOTARY_APPLE_ID=fixture@example.invalid
export AOS_MACOS_NOTARY_APP_PASSWORD=fixture-app-password
export FAKE_CODESIGN_DUMP="$work/ok.dump"
export FAKE_STAPLER_LOG="$work/apple-stapler.log"
sh "$sign" --app "$work/ok.app" --output "$work/apple-out" > "$work/apple.out" 2> "$work/apple.err"
grep -Fxq staple "$FAKE_STAPLER_LOG" || fail 'Apple-ID success did not staple'
grep -Fxq validate "$FAKE_STAPLER_LOG" || fail 'Apple-ID success did not validate'
expect_fail "$work/apple-missing-password" 'AOS_MACOS_NOTARY_APP_PASSWORD is required' \
  env -u AOS_MACOS_NOTARY_APP_PASSWORD sh "$sign" --app "$work/ok.app" --output "$work/apple-no-password"
expect_fail "$work/apple-missing-id" 'AOS_MACOS_NOTARY_APPLE_ID is required' \
  env -u AOS_MACOS_NOTARY_APPLE_ID sh "$sign" --app "$work/ok.app" --output "$work/apple-no-id"
: > "$FAKE_STAPLER_LOG"
expect_fail "$work/apple-rejected" 'fixture notarization rejected' \
  env FAKE_NOTARY_FAIL=1 sh "$sign" --app "$work/ok.app" --output "$work/apple-rejected-out"
[ ! -s "$FAKE_STAPLER_LOG" ] || fail 'Notarization rejection reached stapler'

# Workflow contract: Darwin-only AOS secrets, no Astrid names, no build environment,
# unchanged 6-arg package-release.sh, Command Center path via GITHUB_ENV.
python3 - "$workflow" <<'PY'
from pathlib import Path
import sys

text = Path(sys.argv[1]).read_text(encoding="utf-8")
if "ASTRID_MACOS_" in text:
    raise SystemExit("release.yml must not mention ASTRID_MACOS_ credentials")
if "secrets.ASTRID_" in text:
    raise SystemExit("release.yml must not borrow Astrid secrets")
if "list-keychains" in text or "default-keychain" in text:
    raise SystemExit("release.yml must not mutate the runner keychain search list")

lines = text.splitlines()
build = []
in_build = False
for line in lines:
    if line.startswith("  build:"):
        in_build = True
        build.append(line)
        continue
    if in_build and line[:2] == "  " and not line.startswith("    ") and line.strip().endswith(":"):
        break
    if in_build:
        build.append(line)
if not build:
    raise SystemExit("build job not found")
if any(line.strip().startswith("environment:") for line in build):
    raise SystemExit("build job must not set environment:")

block = "\n".join(build)
required = [
    "name: Sign and staple the Darwin Command Center",
    "if: ${{ endsWith(matrix.target, 'apple-darwin') }}",
    "secrets.AOS_MACOS_DEVELOPMENT_TEAM_ID",
    "secrets.AOS_MACOS_DEVELOPER_ID_IDENTITY",
    "secrets.AOS_MACOS_CERTIFICATE_P12",
    "secrets.AOS_MACOS_CERTIFICATE_PASSWORD",
    "secrets.AOS_MACOS_NOTARY_APPLE_ID",
    "secrets.AOS_MACOS_NOTARY_APP_PASSWORD",
    "unset AOS_MACOS_CERTIFICATE_P12",
    "aos-command-center-signing.p12",
    "AOS_MACOS_CERTIFICATE_P12_PATH",
    'echo "AOS_COMMAND_CENTER_APP=$PROD_OUT/AOS Command Center.app" >> "$GITHUB_ENV"',
    "scripts/sign-command-center.sh",
    "scripts/build-command-center.sh",
]
for item in required:
    if item not in block:
        raise SystemExit(f"build job missing {item!r}")
for obsolete in ("secrets.AOS_MACOS_NOTARY_KEY", "secrets.AOS_MACOS_NOTARY_ISSUER_ID"):
    if obsolete in block:
        raise SystemExit(f"build job still requires unused API credential {obsolete}")

compose_marker = "      - name: Compose product bundle"
if compose_marker not in block:
    raise SystemExit("compose step missing from build job")
if block.index("name: Sign and staple the Darwin Command Center") > block.index("name: Compose product bundle"):
    raise SystemExit("Darwin signing must run before compose")

# The shared compose invocation stays six positional arguments.
expected_compose = """          scripts/package-release.sh \\
            "${{ matrix.target }}" \\
            "$AOS_BINARY" \\
            "$RUNTIME_ASSET" \\
            "$RUNTIME_BLAKE3" \\
            capsule-artifacts \\
            artifacts"""
if expected_compose not in block:
    raise SystemExit("package-release.sh compose arguments changed")
if "AOS_COMMAND_CENTER_APP" not in block.split("name: Compose product bundle", 1)[0]:
    raise SystemExit("AOS_COMMAND_CENTER_APP must be exported before compose")

linux_cells = [
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-musl",
]
sign_step = block.split("name: Sign and staple the Darwin Command Center", 1)[1]
sign_step = sign_step.split("name: Compose product bundle", 1)[0]
for cell in linux_cells:
    if cell in sign_step:
        raise SystemExit(f"Darwin signing step must not mention {cell}")

validate = []
in_validate = False
for line in lines:
    if line.startswith("  validate-release:"):
        in_validate = True
        continue
    if in_validate and line[:2] == "  " and not line.startswith("    ") and line.strip().endswith(":"):
        break
    if in_validate:
        validate.append(line)
if "bash scripts/test_sign_command_center.sh" not in "\n".join(validate):
    raise SystemExit("validate-release must run test_sign_command_center.sh")
PY

# Helper contract: ephemeral import, exact identity, no search-list mutation.
if grep -E -q 'security find-identity|security export' "$sign"; then
  fail "sign-command-center.sh must not consult or export the keychain"
fi
if grep -E -q 'list-keychains|default-keychain' "$sign"; then
  fail "sign-command-center.sh must not mutate the runner keychain search list"
fi
if ! grep -F -q -- '--keychain "$keychain"' "$sign"; then
  fail "sign-command-center.sh must bind codesign to the imported keychain"
fi
if ! grep -F -q 'grep -Fxq "TeamIdentifier=$team_id"' "$sign"; then
  fail "TeamIdentifier check must be an exact line"
fi
if ! grep -F -q 'grep -Fxq "Identifier=$bundle_identifier"' "$sign"; then
  fail "Identifier check must be an exact line"
fi
if ! grep -q 'create-keychain' "$sign" || ! grep -q 'import' "$sign"; then
  fail "sign-command-center.sh must import PKCS12 into an ephemeral keychain"
fi
if grep -q 'ASTRID_MACOS_' "$sign"; then
  :
else
  fail "sign-command-center.sh must detect Astrid credentials in order to refuse them"
fi
if grep -q 'eval.*ASTRID' "$sign"; then
  fail "sign-command-center.sh must not eval Astrid credentials"
fi

echo "sign-command-center fixture and workflow contract checks passed"
