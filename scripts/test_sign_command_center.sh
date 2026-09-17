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
if [ "$1" = "--force" ]; then
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
printf 'accepted\n'
exit 0
SH
  cat >"$dir/stapler" <<'SH'
#!/bin/sh
set -eu
[ "$1" = staple ] || [ "$1" = validate ]
exit 0
SH
  chmod 755 "$dir/codesign" "$dir/ditto" "$dir/xcrun" "$dir/stapler"
  printf '%s\n' "$dump" >"$dir/.dump-path"
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
}

# Missing AOS credentials fail closed before any tool runs.
unset AOS_MACOS_DEVELOPMENT_TEAM_ID AOS_MACOS_DEVELOPER_ID_IDENTITY \
  AOS_MACOS_NOTARY_KEY_PATH AOS_MACOS_NOTARY_KEY_ID AOS_MACOS_NOTARY_ISSUER_ID \
  AOS_MACOS_NOTARY_PROFILE AOS_MACOS_TOOL_DIR || true
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

# Tool wrappers live in AOS_MACOS_TOOL_DIR; PATH must not be used.
tool_dir="$work/tools"
path_trap="$work/path-trap"
mkdir -p "$path_trap"
cat >"$path_trap/codesign" <<'SH'
#!/bin/sh
echo "used PATH codesign" >&2
exit 1
SH
chmod 755 "$path_trap/codesign"
export PATH="$path_trap:$PATH"
export AOS_MACOS_TOOL_DIR="$tool_dir"
aos_env

write_dump "$work/team.dump" "OTHERTEAM1" "ai.unicity.aos.tray"
install_tools "$tool_dir" "$work/team.dump"
make_app "$work/team.app"
FAKE_CODESIGN_DUMP="$work/team.dump" \
expect_fail "$work/team-mismatch" \
  "Command Center TeamIdentifier must match AOS_MACOS_DEVELOPMENT_TEAM_ID" \
  env FAKE_CODESIGN_DUMP="$work/team.dump" \
  sh "$sign" --app "$work/team.app" --output "$work/team-out"

write_dump "$work/id.dump" "TEAMID12AB" "ai.unicity.aos.wrong"
install_tools "$tool_dir" "$work/id.dump"
make_app "$work/id.app"
expect_fail "$work/id-mismatch" \
  "Command Center identifier must remain ai.unicity.aos.tray" \
  env FAKE_CODESIGN_DUMP="$work/id.dump" \
  sh "$sign" --app "$work/id.app" --output "$work/id-out"

# Success fixture: Developer ID dump, ditto zip before notarytool, staple.
write_dump "$work/ok.dump" "TEAMID12AB" "ai.unicity.aos.tray"
install_tools "$tool_dir" "$work/ok.dump"
make_app "$work/ok.app"
printf 'not-a-secret\n' > "$work/notary.p8"
chmod 600 "$work/notary.p8"
signed=$(
  FAKE_CODESIGN_DUMP="$work/ok.dump" \
  AOS_MACOS_NOTARY_KEY_PATH="$work/notary.p8" \
  sh "$sign" --app "$work/ok.app" --output "$work/ok-out"
)
[ "$signed" = "$work/ok-out/AOS Command Center.app" ] || fail "success path did not print the signed app"
[ -d "$signed" ] || fail "signed app missing"
[ ! -L "$signed" ] || fail "signed app became a symlink"
[ -f "$signed/Contents/MacOS/aos-tray" ] || fail "signed app lost its executable"

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
    "secrets.AOS_MACOS_NOTARY_KEY_ID",
    "secrets.AOS_MACOS_NOTARY_ISSUER_ID",
    "secrets.AOS_MACOS_NOTARY_KEY",
    "unset AOS_MACOS_NOTARY_KEY",
    'echo "AOS_COMMAND_CENTER_APP=$PROD_OUT/AOS Command Center.app" >> "$GITHUB_ENV"',
    "scripts/sign-command-center.sh",
    "scripts/build-command-center.sh",
]
for item in required:
    if item not in block:
        raise SystemExit(f"build job missing {item!r}")

compose_marker = "      - name: Compose product bundle"
if compose_marker not in block:
    raise SystemExit("compose step missing from build job")
if block.index("name: Sign and staple the Darwin Command Center") > block.index("name: Compose product bundle"):
    raise SystemExit("Darwin signing must run before compose")

# The shared compose invocation stays six positional arguments.
expected_compose = '''          scripts/package-release.sh \\
            "${{ matrix.target }}" \\
            "$AOS_BINARY" \\
            "$RUNTIME_ASSET" \\
            "$RUNTIME_BLAKE3" \\
            capsule-artifacts \\
            artifacts'''
if expected_compose not in block:
    raise SystemExit("package-release.sh compose arguments changed")
if "AOS_COMMAND_CENTER_APP" not in block.split("name: Compose product bundle", 1)[0]:
    raise SystemExit("AOS_COMMAND_CENTER_APP must be exported before compose")

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

# Helper contract: no keychain export and no Astrid fallback implementation.
if grep -E -q 'security find-identity|security export' "$sign"; then
  fail "sign-command-center.sh must not consult or export the keychain"
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
