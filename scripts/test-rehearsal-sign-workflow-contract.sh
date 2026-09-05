#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
workflow="$repo_root/.github/workflows/rehearsal-sign-darwin.yml"

[[ -f "$workflow" ]]
bash -n "$repo_root/scripts/test-rehearsal-sign-workflow-contract.sh"
grep -Fq 'sealer_pubkey_equals_manifest' "$repo_root/scripts/package-release.sh"
grep -Fq 'if [[ "$sealed" != "$declared" ]]; then' "$repo_root/scripts/package-release.sh"
grep -Fq 'refusing to publish' "$repo_root/scripts/package-release.sh"

grep -Fq 'workflow_dispatch:' "$workflow"
grep -Fq 'github.sha' "$workflow"
grep -Fq 'git fetch --no-tags origin main' "$workflow"
grep -Fq 'git merge-base --is-ancestor "$COMMIT" origin/main' "$workflow"
grep -Fq 'ASTRID_SOURCE_COMMIT: 3de333f1b39d41bacfb535b49d83198212bf2403' "$workflow"
grep -Fq 'ASTRID_RUNTIME_TARGET: aarch64-apple-darwin' "$workflow"
grep -Fq 'submodules: recursive' "$workflow"
grep -Fq -- '--extract-release-sealer' "$workflow"
grep -Fq -- '--sign-release-archive' "$workflow"
grep -Fq 'secrets.token_hex(32)' "$workflow"
grep -Fq 'write_tar_listing' "$workflow"
grep -Fq 'tar -tzf "$archive" > "$listing"' "$workflow"
grep -Fq 'SIGNED_TAR_LISTING=$(mktemp "$RUNNER_TEMP/rehearsal-tar-listing.XXXXXX")' "$workflow"
grep -Fq "grep -q '/Distro.sig$' \"\$SIGNED_TAR_LISTING\"" "$workflow"
if grep -Eq 'tar[[:space:]][^|]*\|[[:space:]]*grep[[:space:]]+(-q|--quiet)' "$workflow"; then
  echo "rehearsal workflow must not pipe tar into grep -q" >&2
  exit 1
fi
grep -Fq 'QA_SEED_FILE="$RUNNER_TEMP/rehearsal-qa-seed"' "$workflow"
[[ $(grep -Fc 'QA_SEED_FILE' "$workflow") -ge 4 ]]
grep -Fq 'actions/upload-artifact@' "$workflow"
grep -Fq 'REHEARSAL-ONLY' "$workflow"

if grep -Eq '^(  )?(push|pull_request|schedule|workflow_call):' "$workflow"; then
  echo "rehearsal workflow must be dispatch-only" >&2
  exit 1
fi
if grep -Fq 'refs/tags/' "$workflow"; then
  echo "rehearsal workflow must not consume a tag ref" >&2
  exit 1
fi
if grep -Fq 'environment: release' "$workflow"; then
  echo "rehearsal workflow must not use the production release environment" >&2
  exit 1
fi
if grep -Fq 'secrets.AOS_DISTRO_ED25519_SEED' "$workflow"; then
  echo "rehearsal workflow must not consume the production seed" >&2
  exit 1
fi
if grep -Fq 'gh release' "$workflow"; then
  echo "rehearsal workflow must not create, upload to, or download from a release" >&2
  exit 1
fi
if grep -Fq 'git push' "$workflow"; then
  echo "rehearsal workflow must not push refs" >&2
  exit 1
fi
if grep -Fiq 'install\.sh' "$workflow"; then
  echo "rehearsal workflow must not consume the installer" >&2
  exit 1
fi
if grep -Fiq 'fskit' "$workflow"; then
  echo "rehearsal workflow must not dispatch or exercise native FSKit" >&2
  exit 1
fi
if grep -Fq 'utH537RuOuqKwjGx/pHIUAkKapyqPUhHpZIVDU6Q0FA=' "$workflow"; then
  echo "rehearsal workflow must not copy the production Distro public key" >&2
  exit 1
fi

python3 - "$workflow" <<'PY'
import pathlib
import sys

text = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8")
required = [
    '[[ "$GITHUB_REF" == refs/heads/main ]]',
    '[[ "$COMMIT" == "${{ github.sha }}" ]]',
    'ASTRID_REPOSITORY: astrid-runtime/astrid',
    "ASTRID_RUNTIME_VERSION: '2026.9.0'",
    'runs-on: macos-latest',
    'runs-on: ubuntu-latest',
    'name: astrid-${{ env.ASTRID_RUNTIME_TARGET }}',
    'name: aos-community-capsules',
    'name: rehearsal-sign-darwin',
    '[[ "$COUNT" -eq 22 ]]',
    'path.unlink()',
    'REHEARSAL-BLAKE3SUMS.txt',
    'REHEARSAL-SHA256SUMS.txt',
    'REHEARSAL-ONLY-identity.json',
]
for needle in required:
    if needle not in text:
        raise SystemExit(f"rehearsal workflow is missing: {needle}")

# package-release.sh performs the fail-closed manifest/public-key check; the
# rehearsal workflow must call both sides through that existing boundary.
for boundary in (
    'package-release.sh" \\\n            --extract-release-sealer',
    'package-release.sh" \\\n            --sign-release-archive',
):
    if boundary not in text:
        raise SystemExit("rehearsal workflow must compose and sign through package-release.sh")

for forbidden in (
    "environment: release",
    "secrets.AOS_DISTRO_ED25519_SEED",
    "gh release",
    "git push",
    "refs/tags/",
    "install.sh",
):
    if forbidden in text:
        raise SystemExit(f"rehearsal workflow contains forbidden text: {forbidden}")
PY
