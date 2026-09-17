#!/bin/sh
# Build an ad-hoc-signed local preview. Never installs or launches it.
set -eu

package_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
swift build --package-path "$package_dir"
binary_dir=$(swift build --package-path "$package_dir" --show-bin-path)
preview="$package_dir/.build/AOS Preview.app"
mkdir -p "$preview/Contents/MacOS"
cp "$package_dir/Info.plist" "$preview/Contents/Info.plist"
cp "$binary_dir/aos-tray" "$preview/Contents/MacOS/aos-tray"
/usr/bin/plutil -lint "$preview/Contents/Info.plist"
/usr/bin/codesign --force --sign - "$preview"
/usr/bin/codesign --verify --strict "$preview"
printf '%s\n' "$preview"
