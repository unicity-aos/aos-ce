#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
validator="$repo_root/scripts/validate-runtime-archive.py"

[[ -f "$validator" ]]

source_mode=$(python3 - "$validator" <<'PY'
import os
import sys

print(oct(os.stat(sys.argv[1]).st_mode & 0o777))
PY
)
[[ "$source_mode" == "0o644" ]]

scratch=$(mktemp -d "${TMPDIR:-/tmp}/rehearsal-validator-XXXXXX")
trap 'rm -rf "$scratch"' EXIT

fixture="$scratch/validate-runtime-archive.py"
archive="$scratch/runtime.tar.gz"
root="astrid-2026.9.0-x86_64-unknown-linux-gnu"
tool="astrid-build"

cp "$validator" "$fixture"
chmod 0644 "$fixture"

fixture_mode=$(python3 - "$fixture" <<'PY'
import os
import sys

print(oct(os.stat(sys.argv[1]).st_mode & 0o777))
PY
)
[[ "$fixture_mode" == "0o644" ]]

python3 - "$archive" "$root" "$tool" <<'PY'
import io
import sys
import tarfile
from pathlib import Path

archive_path = Path(sys.argv[1])
root = sys.argv[2]
tool = sys.argv[3]

with tarfile.open(archive_path, mode="w:gz") as archive:
    directory = tarfile.TarInfo(root)
    directory.type = tarfile.DIRTYPE
    directory.mode = 0o755
    archive.addfile(directory)

    data = b"fixture"
    member = tarfile.TarInfo(f"{root}/{tool}")
    member.mode = 0o755
    member.size = len(data)
    archive.addfile(member, io.BytesIO(data))
PY

python3 "$fixture" "$archive" "$root" "$tool"

set +e
"$fixture" "$archive" "$root" "$tool" >"$scratch/direct.stdout" 2>"$scratch/direct.stderr"
status=$?
set -e

if [[ "$status" -ne 126 ]]; then
  echo "mode-0644 validator direct-exec status was $status, expected 126" >&2
  cat "$scratch/direct.stderr" >&2
  exit 1
fi
