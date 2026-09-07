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
  'runs-on: ${{ matrix.runner }}' \
  'strategy:' \
  'fail-fast: false' \
  '- x86_64-unknown-linux-gnu' \
  '- aarch64-unknown-linux-gnu' \
  'runner: ubuntu-latest' \
  'runner: ubuntu-24.04-arm' \
  'name: rehearsal-sign-darwin' \
  '-name "unicity-aos-2026.9.0-${{ matrix.target }}.tar.gz"' \
  'mapfile -d' \
  'bash scripts/test-packaged-linux-volume.sh' \
  'sudo apt-get install -y --no-install-recommends fuse3 util-linux' \
  'test -e /dev/fuse' \
  'cargo install b3sum --locked --version "$B3SUM_VERSION"' \
  'b3sum'
do
  grep -Fq -- "$required" "$workflow" || {
    echo "packaged Linux volume workflow is missing: $required" >&2
    exit 1
  }
done

python3 - "$workflow" <<'PY'
import pathlib
import re
import sys

text = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8")
matches = list(re.finditer(r"(?m)^  ([A-Za-z0-9_-]+):\n", text))
sections = {
    match.group(1): text[match.start(): (
        matches[index + 1].start() if index + 1 < len(matches) else len(text)
    )]
    for index, match in enumerate(matches)
}
job = sections.get("packaged-linux-volume")
if job is None:
    raise SystemExit("packaged Linux volume job is missing")

matrix_match = re.search(r"(?ms)^    strategy:\n.*?^    steps:\n", job)
if matrix_match is None:
    raise SystemExit("packaged Linux volume job is not matrixed")
matrix = matrix_match.group(0)
for required in (
    "fail-fast: false",
    "target:",
    "- x86_64-unknown-linux-gnu",
    "- aarch64-unknown-linux-gnu",
    "include:",
    "- target: x86_64-unknown-linux-gnu",
    "runner: ubuntu-latest",
    "- target: aarch64-unknown-linux-gnu",
    "runner: ubuntu-24.04-arm",
):
    if required not in matrix:
        raise SystemExit(f"packaged Linux volume matrix is missing: {required}")
for required in (
    "runs-on: ${{ matrix.runner }}",
    '-name "unicity-aos-2026.9.0-${{ matrix.target }}.tar.gz"',
):
    if required not in job:
        raise SystemExit(f"packaged Linux volume job is missing: {required}")
PY

for required in \
  'x86_64) target=x86_64-unknown-linux-gnu ;;' \
  'aarch64) target=aarch64-unknown-linux-gnu ;;' \
  'AOS_PACKAGED_TARGET' \
  'x86_64-unknown-linux-gnu | x86_64-unknown-linux-musl' \
  'aarch64-unknown-linux-gnu | aarch64-unknown-linux-musl' \
  'expected_root=unicity-aos-2026.9.0-${target}' \
  'REHEARSAL_TARGET=$target' \
  'REHEARSAL-ONLY-identity.json' \
  'REHEARSAL-BLAKE3SUMS.txt' \
  'REHEARSAL-SHA256SUMS.txt' \
  'signed rehearsal artifact is missing identity/checksum manifest' \
  'GITHUB_SHA' \
  'archive identity does not bind the exact AOS workflow commit' \
  'stream.extractall(destination, filter="data")' \
  'agent create operator-qa --group agent --yes' \
  'agent show operator-qa --format json' \
  'distro apply --principal operator-qa --yes --offline' \
  'Installation incomplete:' \
  'for pass in 1 2 3; do' \
  'active_receipt="$work/home/.aos/receipts/unicity-ce.active.json"' \
  'ASTRID_PRINCIPAL=default ASTRID_VAR_OPENAI_API_KEY=' \
  'sleep 61' \
  'run_default status --json' \
  'run_default stop' \
  '${#work} > 34' \
  'mktemp -d /tmp/aos-pv.XXXXXX' \
  'exactly 22 ready capsules' \
  'unsafe cleanup; preserving disposable evidence' \
  'runner_image=' \
  'command -v fusermount3' \
  'apk info -e fuse3' \
  'storage mount --as operator-qa --read-write' \
  'findmnt -n -o FSTYPE --mountpoint' \
  'mount_is_active' \
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
