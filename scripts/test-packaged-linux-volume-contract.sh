#!/usr/bin/env bash
# shellcheck disable=SC2016 # grep needles intentionally contain shell syntax.
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
workflow="$repo_root/.github/workflows/rehearsal-sign-darwin.yml"
journey="$repo_root/scripts/test-packaged-linux-volume.sh"
[[ -f "$workflow" && -f "$journey" ]]
bash -n "$journey"

for required in \
  'packaged-linux-volume:' \
  'needs: compose-and-sign' \
  'runs-on: ubuntu-latest' \
  'name: rehearsal-sign-darwin' \
  "-name 'unicity-aos-2026.9.0-x86_64-unknown-linux-gnu.tar.gz'" \
  'mapfile -d' \
  'bash scripts/test-packaged-linux-volume.sh' \
  'sudo apt-get install -y --no-install-recommends fuse3 util-linux' \
  'test -e /dev/fuse'
do
  grep -Fq -- "$required" "$workflow" || {
    echo "packaged Linux volume workflow is missing: $required" >&2
    exit 1
  }
done

for required in \
  'expected_root=unicity-aos-2026.9.0-x86_64-unknown-linux-gnu' \
  'stream.extractall(destination, filter="data")' \
  'distro apply --principal operator-qa --yes --offline' \
  'storage mount --as operator-qa --read-write' \
  'findmnt -n -o FSTYPE --target' \
  '/proc/self/mountinfo' \
  'storage status' \
  'storage sync' \
  'storage unmount' \
  'astrid.volume' \
  'stat -c' \
  'rm -rf "$work"'
do
  grep -Fq -- "$required" "$journey" || {
    echo "packaged Linux volume journey is missing: $required" >&2
    exit 1
  }
done

# The journey must consume an archive and the packaged executables. It must not
# silently fall back to source builds, installer downloads, or a live home.
if grep -Eq 'cargo[[:space:]]+build|install\.sh|git[[:space:]]+clone|~/(\.aos|\.astrid)' "$journey"; then
  echo "packaged Linux volume journey contains a source/install/live-home fallback" >&2
  exit 1
fi
for forbidden in 'gh release' 'git push' 'ASTRID_HOME=$HOME'; do
  if grep -Fq "$forbidden" "$workflow" "$journey"; then
    echo "packaged Linux volume rehearsal contains forbidden mutation: $forbidden" >&2
    exit 1
  fi
done
echo "packaged Linux volume workflow contract passed"
