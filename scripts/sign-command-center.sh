#!/bin/sh
# Developer ID sign, notarize, and staple an assembled AOS Command Center.app.
# Production validation stays in build-command-center.sh --mode production.
# This helper never reads ASTRID_* credentials and never exports key material.
set -eu

usage() {
  echo "usage: $0 --app PATH --output DIR" >&2
  exit 2
}

bundle_name="AOS Command Center.app"
bundle_identifier="ai.unicity.aos.tray"
app=
output_dir=

while [ $# -gt 0 ]; do
  case "$1" in
    --app)
      [ $# -ge 2 ] || usage
      app=$2
      shift 2
      ;;
    --output)
      [ $# -ge 2 ] || usage
      output_dir=$2
      shift 2
      ;;
    *)
      usage
      ;;
  esac
done

[ -n "$app" ] || usage
[ -n "$output_dir" ] || usage

require_env() {
  name=$1
  eval "value=\${$name-}"
  if [ -z "$value" ]; then
    echo "$name is required to Developer ID sign a Command Center app" >&2
    exit 1
  fi
}

require_tool() {
  path=$1
  name=$2
  if [ ! -x "$path" ]; then
    echo "$name is required to $3 a Command Center app" >&2
    exit 1
  fi
}

if [ -n "${ASTRID_MACOS_DEVELOPMENT_TEAM_ID-}" ] || \
   [ -n "${ASTRID_MACOS_DEVELOPER_ID_IDENTITY-}" ] || \
   [ -n "${ASTRID_MACOS_NOTARY_KEY-}" ] || \
   [ -n "${ASTRID_MACOS_NOTARY_KEY_ID-}" ] || \
   [ -n "${ASTRID_MACOS_NOTARY_ISSUER_ID-}" ] || \
   [ -n "${ASTRID_FSKIT_DEVELOPMENT_TEAM-}" ] || \
   [ -n "${ASTRID_FSKIT_CODE_SIGN_IDENTITY-}" ]; then
  if [ -z "${AOS_MACOS_DEVELOPMENT_TEAM_ID-}" ] || \
     [ -z "${AOS_MACOS_DEVELOPER_ID_IDENTITY-}" ]; then
    echo "Command Center signing does not borrow Astrid credentials" >&2
    exit 1
  fi
fi

require_env AOS_MACOS_DEVELOPMENT_TEAM_ID
require_env AOS_MACOS_DEVELOPER_ID_IDENTITY
team_id=$AOS_MACOS_DEVELOPMENT_TEAM_ID
identity=$AOS_MACOS_DEVELOPER_ID_IDENTITY

if [ "$identity" = "-" ] || [ "$identity" = "adhoc" ] || [ "$identity" = "ad-hoc" ]; then
  echo "Command Center signing refuses an ad-hoc identity" >&2
  exit 1
fi

if [ ! -d "$app" ] || [ -L "$app" ]; then
  echo "Command Center app must be a regular directory: $app" >&2
  exit 1
fi
case "$app" in
  *.app) ;;
  *)
    echo "Command Center app path must end with .app" >&2
    exit 1
    ;;
esac

tool_dir=${AOS_MACOS_TOOL_DIR:-/usr/bin}
codesign="$tool_dir/codesign"
ditto="$tool_dir/ditto"
xcrun="$tool_dir/xcrun"
stapler="$tool_dir/stapler"
require_tool "$codesign" codesign "Developer ID sign"
require_tool "$ditto" ditto "zip"
require_tool "$xcrun" xcrun "notarize"
require_tool "$stapler" stapler "staple"

destination="$output_dir/$bundle_name"
mkdir -p "$output_dir"
if [ -e "$destination" ] || [ -L "$destination" ]; then
  echo "Command Center output already exists: $destination" >&2
  exit 1
fi
cp -Rp "$app" "$destination"
if [ -L "$destination" ]; then
  echo "Command Center copy must not be a symlink" >&2
  exit 1
fi

"$codesign" --force --options runtime --timestamp \
  --identifier "$bundle_identifier" \
  --sign "$identity" \
  --deep \
  "$destination"
if ! "$codesign" --verify --strict --deep "$destination"; then
  echo "Command Center is not a valid signed macOS app after Developer ID sign" >&2
  exit 1
fi

work=$(mktemp -d)
dump="$work/codesign.dump"
zip="$work/command-center-notary.zip"
trap 'rm -rf "$work"' EXIT
"$codesign" -d --verbose=2 "$destination" >"$dump" 2>&1 || true
if grep -q 'Signature=adhoc' "$dump"; then
  echo "Command Center must not remain ad-hoc signed" >&2
  exit 1
fi
if ! grep -q 'Authority=Developer ID Application' "$dump"; then
  echo "Command Center must be Developer ID Application signed" >&2
  exit 1
fi
if ! grep -q "TeamIdentifier=$team_id" "$dump"; then
  echo "Command Center TeamIdentifier must match AOS_MACOS_DEVELOPMENT_TEAM_ID" >&2
  exit 1
fi
if ! grep -q "Identifier=$bundle_identifier" "$dump"; then
  echo "Command Center identifier must remain ai.unicity.aos.tray" >&2
  exit 1
fi

"$ditto" -c -k --keepParent "$destination" "$zip"
if [ -n "${AOS_MACOS_NOTARY_PROFILE-}" ]; then
  "$xcrun" notarytool submit "$zip" \
    --keychain-profile "$AOS_MACOS_NOTARY_PROFILE" --wait >&2
else
  require_env AOS_MACOS_NOTARY_KEY_PATH
  require_env AOS_MACOS_NOTARY_KEY_ID
  require_env AOS_MACOS_NOTARY_ISSUER_ID
  if [ ! -f "$AOS_MACOS_NOTARY_KEY_PATH" ] || [ -L "$AOS_MACOS_NOTARY_KEY_PATH" ]; then
    echo "AOS_MACOS_NOTARY_KEY_PATH must be a regular file" >&2
    exit 1
  fi
  "$xcrun" notarytool submit "$zip" \
    --key "$AOS_MACOS_NOTARY_KEY_PATH" \
    --key-id "$AOS_MACOS_NOTARY_KEY_ID" \
    --issuer "$AOS_MACOS_NOTARY_ISSUER_ID" \
    --wait >&2
fi

"$stapler" staple "$destination" >&2
"$stapler" validate "$destination" >&2
printf '%s\n' "$destination"
