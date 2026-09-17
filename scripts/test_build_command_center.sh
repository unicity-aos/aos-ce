#!/usr/bin/env bash
# Production builder fail-closed checks. Does not swift-build or codesign --sign.
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

if bash "$repo_root/scripts/build-command-center.sh" \
  --mode production --output "$work/missing-app" >/dev/null 2>"$work/missing-app.err"; then
  echo "production builder accepted a missing --app" >&2
  exit 1
fi
grep -Fq -- "--app PATH" "$work/missing-app.err"

PYTHONPATH="$repo_root/scripts" python3 -c \
  'from pathlib import Path; import sys; from test_package_macos_command_center import fixture; fixture(Path(sys.argv[1]))' \
  "$work/unsigned.app"

if bash "$repo_root/scripts/build-command-center.sh" \
  --mode production --app "$work/unsigned.app" --output "$work/unsigned-out" \
  >/dev/null 2>"$work/unsigned.err"; then
  echo "production builder accepted an unsigned Command Center fixture" >&2
  exit 1
fi
if grep -Eq 'codesign is required|stapler is required|not a valid signed|must not be ad-hoc|Developer ID Application' \
  "$work/unsigned.err"; then
  :
else
  echo "production builder did not fail closed on an unsigned fixture" >&2
  cat "$work/unsigned.err" >&2
  exit 1
fi

echo "build-command-center production fail-closed checks passed"
