#!/usr/bin/env bash
set -euo pipefail

realm_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
mode=package
builder="$realm_root/build-output/aos-linux-realm.capsule"

while [[ $# -gt 0 ]]; do
  case $1 in
    --check)
      mode=check
      ;;
    --builder)
      [[ $# -ge 2 ]] || { echo "missing path after --builder" >&2; exit 2; }
      builder=$2
      shift
      ;;
    *)
      echo "usage: $0 [--builder BUILDER_ARCHIVE] [--check]" >&2
      exit 2
      ;;
  esac
  shift
done

if [[ "$mode" == "check" ]]; then
  exec python3 "$realm_root/scripts/validate_capsule.py"
fi

if [[ ! -f "$builder" ]]; then
  echo "missing pinned builder archive: $builder" >&2
  echo "run 'aos capsule build --output build-output' first" >&2
  exit 1
fi

export COPYFILE_DISABLE=1
export SOURCE_DATE_EPOCH=0
exec python3 "$realm_root/scripts/canonical_package.py" --builder "$builder"
