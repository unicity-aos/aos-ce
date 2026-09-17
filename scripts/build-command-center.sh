#!/bin/sh
# Assemble a versioned AOS Command Center.app for Darwin packaging.
# Development may ad-hoc sign for local tests. Production never signs, notarizes,
# or uses keys; it copies an already Developer ID signed and stapled app.
set -eu

usage() {
  echo "usage: $0 --mode development --target aarch64-apple-darwin|x86_64-apple-darwin --output DIR" >&2
  echo "usage: $0 --mode production --app PATH --output DIR" >&2
  exit 2
}

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
package_dir="$repo_root/apps/aos-tray"
bundle_name="AOS Command Center.app"
mode=
target=
app=
output_dir=

while [ $# -gt 0 ]; do
  case "$1" in
    --mode)
      [ $# -ge 2 ] || usage
      mode=$2
      shift 2
      ;;
    --target)
      [ $# -ge 2 ] || usage
      target=$2
      shift 2
      ;;
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

[ -n "$mode" ] || usage
[ -n "$output_dir" ] || usage

product_version() {
  python3 - "$repo_root/crates/unicity-aos-bootstrap/Cargo.toml" <<'PY'
import pathlib
import re
import sys

text = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8")
match = re.search(r'(?m)^version\s*=\s*"([^"]+)"', text)
if match is None:
    raise SystemExit("product version is missing from unicity-aos-bootstrap")
print(match.group(1))
PY
}

require_tool() {
  path=$1
  name=$2
  if [ ! -x "$path" ]; then
    echo "$name is required to $3 a Command Center app" >&2
    exit 1
  fi
}

destination="$output_dir/$bundle_name"
mkdir -p "$output_dir"
if [ -e "$destination" ] || [ -L "$destination" ]; then
  echo "Command Center output already exists: $destination" >&2
  exit 1
fi

if [ "$mode" = development ]; then
  [ -z "$app" ] || usage
  case "$target" in
    aarch64-apple-darwin) arch=arm64 ;;
    x86_64-apple-darwin) arch=x86_64 ;;
    *) usage ;;
  esac
  require_tool /usr/bin/swift swift "build"
  require_tool /usr/bin/plutil plutil "stamp"
  require_tool /usr/bin/codesign codesign "ad-hoc sign"
  version=$(product_version)
  /usr/bin/swift build --package-path "$package_dir" --arch "$arch" -c release
  binary_dir=$(/usr/bin/swift build --package-path "$package_dir" --arch "$arch" -c release --show-bin-path)
  mkdir -p "$destination/Contents/MacOS"
  cp "$package_dir/Info.plist" "$destination/Contents/Info.plist"
  cp "$binary_dir/aos-tray" "$destination/Contents/MacOS/aos-tray"
  chmod 755 "$destination/Contents/MacOS/aos-tray"
  /usr/bin/plutil -replace CFBundleDisplayName -string "AOS Command Center" \
    "$destination/Contents/Info.plist"
  /usr/bin/plutil -replace CFBundleName -string "AOS Command Center" \
    "$destination/Contents/Info.plist"
  /usr/bin/plutil -replace CFBundleShortVersionString -string "$version" \
    "$destination/Contents/Info.plist"
  /usr/bin/plutil -replace CFBundleVersion -string "$version" \
    "$destination/Contents/Info.plist"
  /usr/bin/plutil -lint "$destination/Contents/Info.plist" >/dev/null
  /usr/bin/codesign --force --sign - --deep "$destination"
  /usr/bin/codesign --verify --strict --deep "$destination"
  printf '%s\n' "$destination"
  exit 0
fi

if [ "$mode" = production ]; then
  [ -z "$target" ] || usage
  [ -n "$app" ] || usage
  if [ ! -d "$app" ] || [ -L "$app" ]; then
    echo "production Command Center app must be a regular directory: $app" >&2
    exit 1
  fi
  require_tool /usr/bin/codesign codesign "verify"
  require_tool /usr/bin/stapler stapler "validate notarization of"
  cp -Rp "$app" "$destination"
  if [ -L "$destination" ]; then
    echo "production Command Center copy must not be a symlink" >&2
    exit 1
  fi
  if ! /usr/bin/codesign --verify --strict --deep "$destination"; then
    echo "production Command Center is not a valid signed macOS app" >&2
    exit 1
  fi
  dump=$(mktemp)
  trap 'rm -f "$dump"' EXIT
  /usr/bin/codesign -d --verbose=2 "$destination" >"$dump" 2>&1 || true
  if grep -q 'Signature=adhoc' "$dump"; then
    echo "production Command Center must not be ad-hoc signed" >&2
    exit 1
  fi
  if ! grep -q 'Authority=Developer ID Application' "$dump"; then
    echo "production Command Center must be Developer ID Application signed" >&2
    exit 1
  fi
  /usr/bin/stapler validate "$destination"
  printf '%s\n' "$destination"
  exit 0
fi

usage
