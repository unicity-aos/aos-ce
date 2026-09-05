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
for stale_binding in \
  '${darwin_runtime[0]}' \
  '${linux_runtime[0]}' \
  '${darwin_aos[0]}' \
  '${linux_aos[0]}'
do
  if grep -Fq "$stale_binding" "$workflow"; then
    echo "rehearsal workflow contains stale artifact binding: $stale_binding" >&2
    exit 1
  fi
done
for scalar_binding in \
  'DARWIN_RUNTIME_ARCHIVE=$DARWIN_RUNTIME_ARCHIVE' \
  'LINUX_RUNTIME_ARCHIVE=$LINUX_RUNTIME_ARCHIVE' \
  'DARWIN_AOS_BINARY=$DARWIN_AOS_BINARY' \
  'LINUX_AOS_BINARY=$LINUX_AOS_BINARY'
do
  grep -Fq "$scalar_binding" "$workflow"
done
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

identity = sections.get("prepare-rehearsal-identity")
if identity is None:
    raise SystemExit("rehearsal workflow is missing prepare-rehearsal-identity")
if "pubkey: ${{ steps.identity.outputs.pubkey }}" not in identity:
    raise SystemExit("ephemeral QA public key must be the reusable identity output")
if 'astrid-version = "=2026.9.0"' not in identity:
    raise SystemExit("identity overlays must set Distro astrid-version to 2026.9.0")
if "$overlays/runtime-compatibility.toml" not in identity:
    raise SystemExit("identity job must publish runtime-compatibility.toml")
if "$overlays/Distro.toml" not in identity:
    raise SystemExit("identity job must publish Distro.toml")
if "$overlays/distro_trust.rs" not in identity:
    raise SystemExit("identity job must publish the ASTRID_RUNTIME_VERSION overlay")
if 'pub(crate) const ASTRID_RUNTIME_VERSION: &str = "0.10.4";' not in identity:
    raise SystemExit("identity job must bind the production ASTRID_RUNTIME_VERSION constant")
if 'pub(crate) const ASTRID_RUNTIME_VERSION: &str = "2026.9.0";' not in identity:
    raise SystemExit("identity job must overlay ASTRID_RUNTIME_VERSION to 2026.9.0")
if "rehearsal-qa-seed" not in identity:
    raise SystemExit("identity job must persist its private seed")

capsules = sections.get("build-capsules")
if capsules is None:
    raise SystemExit("rehearsal workflow is missing build-capsules")
build_index = capsules.index("scripts/build-capsule-assets.sh")
for marker in (
    "python-version: '3.12'",
    "toolchain: '1.95.0'",
    "targets: wasm32-unknown-unknown",
):
    marker_index = capsules.find(marker)
    if marker_index < 0:
        raise SystemExit(f"capsule job is missing {marker}")
    if marker_index > build_index:
        raise SystemExit("capsule toolchain must be installed before the capsule build")

for job_name in ("build-aos-darwin-binary", "build-aos-linux-gnu-binary"):
    job = sections.get(job_name)
    if job is None:
        raise SystemExit(f"rehearsal workflow is missing {job_name}")
    if "prepare-rehearsal-identity" not in job:
        raise SystemExit(f"{job_name} must wait for rehearsal overlays")
    for marker in (
        "name: rehearsal-overlays",
        'git worktree add --detach "$BUILD_CHECKOUT" "$SOURCE_COMMIT"',
        "rehearsal-overlays/runtime-compatibility.toml",
        "rehearsal-overlays/Distro.toml",
        "rehearsal-overlays/distro_trust.rs",
        "cargo build",
    ):
        if marker not in job:
            raise SystemExit(f"{job_name} is missing {marker}")
    if not (
        job.index("name: rehearsal-overlays")
        < job.index("rehearsal-overlays/runtime-compatibility.toml")
        < job.index("rehearsal-overlays/Distro.toml")
        < job.index("rehearsal-overlays/distro_trust.rs")
        < job.index("cargo build")
    ):
        raise SystemExit(f"{job_name} must install runtime, Distro, and ASTRID_RUNTIME_VERSION overlays before compiling AOS")

compose = sections.get("compose-and-sign")
if compose is None:
    raise SystemExit("rehearsal workflow is missing compose-and-sign")
for marker in ("name: rehearsal-overlays", "name: rehearsal-qa-seed", "QA_PUBKEY"):
    if marker not in compose:
        raise SystemExit(f"compose job is missing {marker}")
if 'astrid-version"] != "=2026.9.0"' not in compose:
    raise SystemExit("compose job must validate Distro astrid-version")
if 'ASTRID_RUNTIME_VERSION: &str = "2026.9.0"' not in compose:
    raise SystemExit("compose job must validate the compile-time ASTRID_RUNTIME_VERSION overlay")
if 'ASTRID_RUNTIME_VERSION: &str = "0.10.4"' not in compose:
    raise SystemExit("compose job must reject a leftover production ASTRID_RUNTIME_VERSION overlay")
if 'runtime["version"] != "2026.9.0"' not in compose:
    raise SystemExit("compose job must validate runtime-compatibility version")
if "Prove overlay-built GNU AOS accepts the signed Distro" not in compose:
    raise SystemExit("compose job must execute overlay-built AOS against the signed Distro")
probe = '"$LINUX_AOS_BINARY" distro apply --principal operator-qa --yes'
if probe not in compose:
    raise SystemExit("compose job must run overlay-built GNU AOS distro apply as the consume probe")
if "rehearsal-consume-aos" not in compose:
    raise SystemExit("compose job must plant the signed Distro into a disposable AOS_HOME")
if "bundled Distro Apply verification failed" not in compose:
    raise SystemExit("compose job must fail closed if overlay-built AOS rejects the signed Distro")
if "failed to start bundled runtime for Distro Apply" not in compose:
    raise SystemExit("compose job must stop the Distro probe before runtime start")
if "for member in Distro.toml Distro.lock Distro.sig release-manifest.json" not in compose:
    raise SystemExit("compose job must plant exact signed Distro members and release-manifest.json")
if "install -m 0600" not in compose:
    raise SystemExit("compose job must plant signed Distro members with private mode 0600")
if '"$DARWIN_AOS_BINARY" distro apply' in compose:
    raise SystemExit("compose job must not execute the Darwin AOS binary on the GNU consume probe")
if not (
    compose.index("--sign-release-archive")
    < compose.index("Prove overlay-built GNU AOS accepts the signed Distro")
    < compose.index(probe)
    < compose.index("Assemble rehearsal-only evidence")
):
    raise SystemExit("compose job must prove overlay-built AOS after signing and before evidence upload")

if re.search(r"QA_SEED[^\n]*GITHUB_(?:ENV|OUTPUT)", text):
    raise SystemExit("ephemeral QA seed must never be emitted")
PY

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
