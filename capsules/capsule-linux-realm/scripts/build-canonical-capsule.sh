#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" -lt 1 || "$#" -gt 2 ]]; then
  echo "usage: $0 BUILDER_ARCHIVE [OUTPUT_CAPSULE]" >&2
  exit 64
fi

builder=$1
output=${2:-"$PWD/dist/aos-linux-realm.capsule"}
realm_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

if [[ ! -x "$builder" && ! -f "$builder" ]]; then
  echo "missing pinned builder archive: $builder" >&2
  exit 66
fi

export COPYFILE_DISABLE=1
export SOURCE_DATE_EPOCH=0

validator_args=()
if [[ -n "${AOS_SOURCE_SHA:-}" ]]; then
  validator_args=(--source-sha "$AOS_SOURCE_SHA")
fi

python3 "$realm_root/scripts/canonical_package.py" \
  --builder "$builder" \
  --output "$output"
python3 "$realm_root/scripts/validate_capsule.py" "$output" "${validator_args[@]}"
