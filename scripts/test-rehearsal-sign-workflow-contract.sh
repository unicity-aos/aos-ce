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
grep -Fq 'ASTRID_SOURCE_COMMIT: e560b6cda3df59ace1665dd26768a9209fce81ce' "$workflow"
grep -Fq 'tag = "rehearsal-only-e560b6cda3df59ace1665dd26768a9209fce81ce"' "$workflow"
grep -Fq '@workflow_dispatch/e560b6cda3df59ace1665dd26768a9209fce81ce"' "$workflow"
grep -Fq 'source-commit = "e560b6cda3df59ace1665dd26768a9209fce81ce"' "$workflow"
grep -Fq 'ASTRID_RUNTIME_TARGET: aarch64-apple-darwin' "$workflow"
grep -Fq 'submodules: recursive' "$workflow"
grep -Fq -- '--extract-release-sealer' "$workflow"
grep -Fq -- '--sign-release-archive' "$workflow"
if [[ $(grep -Fc -- '--sign-release-archive' "$workflow") -ne 2 ]]; then
  echo "rehearsal workflow must sign exactly one Darwin and one GNU archive" >&2
  exit 1
fi
if [[ $(grep -Fc 'AOS_DISTRO_ED25519_SEED="$QA_SEED"' "$workflow") -ne 2 ]]; then
  echo "Darwin and GNU rehearsal archives must share one ephemeral QA seed" >&2
  exit 1
fi
if [[ $(grep -Fc '"$NATIVE_SEALER" \' "$workflow") -ne 2 ]] || \
   [[ $(grep -Fc '"$NATIVE_SEALER_ARCHIVE"' "$workflow") -ne 2 ]]; then
  echo "Darwin and GNU rehearsal archives must share the authenticated GNU native sealer" >&2
  exit 1
fi
grep -Fq 'secrets.token_hex(32)' "$workflow"
grep -Fq 'write_tar_listing' "$workflow"
grep -Fq 'tar -tzf "$archive" > "$listing"' "$workflow"
grep -Fq 'SIGNED_DARWIN_TAR_LISTING=$(mktemp "$RUNNER_TEMP/rehearsal-darwin-tar-listing.XXXXXX")' "$workflow"
grep -Fq 'SIGNED_GNU_TAR_LISTING=$(mktemp "$RUNNER_TEMP/rehearsal-gnu-tar-listing.XXXXXX")' "$workflow"
grep -Fq "grep -q '/Distro.sig$' \"\$SIGNED_DARWIN_TAR_LISTING\"" "$workflow"
grep -Fq "grep -q '/Distro.sig$' \"\$SIGNED_GNU_TAR_LISTING\"" "$workflow"
grep -Fq 'SIGNED_DARWIN_ARCHIVE=$signed_darwin' "$workflow"
grep -Fq 'SIGNED_GNU_ARCHIVE=$signed_gnu' "$workflow"
grep -Fq 'LINUX_ARCHIVE=${candidates[0]}' "$workflow"
grep -Fq 'ephemeral QA seed destroyed after both signed archive verifications' "$workflow"
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
grep -Fq 'bash scripts/bind-rehearsal-consumed-artifacts.sh' "$workflow"
bash -n "$repo_root/scripts/bind-rehearsal-consumed-artifacts.sh"
grep -Fq 'print_stat' "$repo_root/scripts/bind-rehearsal-consumed-artifacts.sh"
grep -Fq 'chmod 0755' "$repo_root/scripts/bind-rehearsal-consumed-artifacts.sh"
bash "$repo_root/scripts/test-bind-rehearsal-consumed-artifacts.sh"
validator_harness="$repo_root/scripts/test-invoke-runtime-archive-validator.sh"
[[ -f "$validator_harness" ]]
bash -n "$validator_harness"
bash "$validator_harness"
if [[ $(grep -Fc 'python3 "$REHEARSAL_CHECKOUT/scripts/validate-runtime-archive.py"' "$workflow") -ne 2 ]]; then
  echo "rehearsal workflow must invoke the runtime archive validator with python3 at both compose sites" >&2
  exit 1
fi
if grep -Eq '^[[:space:]]*"\$REHEARSAL_CHECKOUT/scripts/validate-runtime-archive\.py"' "$workflow"; then
  echo "rehearsal workflow must not direct-exec the runtime archive validator" >&2
  exit 1
fi
if grep -Fq '[[ -f "$DARWIN_AOS_BINARY" && -x "$DARWIN_AOS_BINARY" ]]' "$workflow"; then
  echo "rehearsal workflow must not assert execute bits before chmod" >&2
  exit 1
fi
if grep -Fq '[[ -f "$LINUX_AOS_BINARY" && -x "$LINUX_AOS_BINARY" ]]' "$workflow"; then
  echo "rehearsal workflow must not assert execute bits before chmod" >&2
  exit 1
fi
grep -Fq 'QA_SEED_FILE="$RUNNER_TEMP/rehearsal-qa-seed"' "$workflow"
[[ $(grep -Fc 'QA_SEED_FILE' "$workflow") -ge 4 ]]
grep -Fq 'actions/upload-artifact@' "$workflow"
grep -Fq 'REHEARSAL-ONLY' "$workflow"

# The uploaded checksum manifest is consumed by b3sum itself. Keep the
# manifest records-only: b3sum does not accept the explanatory comment that
# older rehearsal output prepended. Exercise the real verifier rather than
# relying on a text-only workflow assertion.
checksum_fixture=$(mktemp -d "${TMPDIR:-/tmp}/rehearsal-checksum.XXXXXX")
printf 'darwin rehearsal archive\n' > "$checksum_fixture/aarch64-apple-darwin.tar.gz"
printf 'gnu rehearsal archive\n' > "$checksum_fixture/x86_64-unknown-linux-gnu.tar.gz"
(
  cd "$checksum_fixture"
  b3sum -- aarch64-apple-darwin.tar.gz x86_64-unknown-linux-gnu.tar.gz > \
    REHEARSAL-BLAKE3SUMS.txt
  b3sum --check REHEARSAL-BLAKE3SUMS.txt
)
grep -Fq 'b3sum --check REHEARSAL-BLAKE3SUMS.txt' "$workflow"
grep -Fq 'sha256sum --check REHEARSAL-SHA256SUMS.txt' "$workflow"
if grep -Fq 'echo "# REHEARSAL-ONLY digests; not publication checksums"' "$workflow"; then
  echo "rehearsal checksum manifests must contain records only; keep notes in identity JSON" >&2
  exit 1
fi

python3 - "$workflow" <<'PY'
import pathlib
import sys

text = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8")
if '"checksum_manifest_note": (' not in text:
    raise SystemExit("rehearsal identity must explain the records-only checksum scope")
if '"Checksum manifests contain machine-readable records only."' not in text:
    raise SystemExit("rehearsal identity must preserve the checksum format explanation")
PY

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
if grep -Fq 'manage-macos-fskit.sh' "$workflow" || \
   grep -Fq -- '--astrid-provider-stdio-v1' "$workflow"; then
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

sign_call_archives = re.findall(
    r'AOS_DISTRO_ED25519_SEED="\$QA_SEED"\s+\\\s*'
    r'"\$REHEARSAL_CHECKOUT/scripts/package-release\.sh"\s+\\\s*'
    r'--sign-release-archive\s+\\\s*("\$[A-Z_]+")',
    compose,
)
if sign_call_archives != ['"$DARWIN_ARCHIVE"', '"$LINUX_ARCHIVE"']:
    raise SystemExit(
        "rehearsal workflow must make exactly one shared-identity Darwin sign call followed by one GNU sign call"
    )
if compose.count('"$NATIVE_SEALER"') != 2 or compose.count('"$NATIVE_SEALER_ARCHIVE"') != 2:
    raise SystemExit("both target sign calls must use the authenticated GNU native sealer")
for marker in (
    'SIGNED_DARWIN_ARCHIVE=$signed_darwin',
    'SIGNED_GNU_ARCHIVE=$signed_gnu',
    'grep -q \'/Distro.sig$\' "$SIGNED_DARWIN_TAR_LISTING"',
    'grep -q \'/Distro.sig$\' "$SIGNED_GNU_TAR_LISTING"',
):
    if marker not in compose:
        raise SystemExit(f"compose job is missing shared-identity archive evidence marker: {marker}")
darwin_verify = compose.index("signed Darwin rehearsal archive has no Distro signature")
gnu_verify = compose.index("signed GNU rehearsal archive has no Distro signature")
seed_zero = compose.index('file.write(b"\\0" * 32)')
seed_flush = compose.index("file.flush()", seed_zero)
seed_fsync = compose.index("os.fsync(file.fileno())", seed_flush)
seed_unlink = compose.index("path.unlink()", seed_fsync)
seed_absence = compose.index('[[ ! -e "$QA_SEED_FILE" ]]', seed_unlink)
if not (
    darwin_verify
    < gnu_verify
    < seed_zero
    < seed_flush
    < seed_fsync
    < seed_unlink
    < seed_absence
):
    raise SystemExit(
        "persistent QA seed zero-write/fsync/unlink/absence proof must follow both archive verifications"
    )

darwin_runtime = sections.get("build-astrid-darwin")
if darwin_runtime is None:
    raise SystemExit("rehearsal workflow is missing build-astrid-darwin")
for marker in (
    "-p astrid-storage-provider-fskit",
    "astrid-storage-provider-fskit",
):
    if marker not in darwin_runtime:
        raise SystemExit(f"Darwin runtime job is missing {marker}")

validator_call = 'python3 "$REHEARSAL_CHECKOUT/scripts/validate-runtime-archive.py"'
for step_name in ("Compose the GNU native sealer source", "Compose the Darwin candidate"):
    step = re.search(
        rf"(?ms)^      - name: {re.escape(step_name)}\n(?P<body>.*?)(?=^      - name:|\Z)",
        compose,
    )
    if step is None:
        raise SystemExit(f"compose job is missing {step_name}")
    body = step.group("body")
    if body.count(validator_call) != 1:
        raise SystemExit(f"{step_name} must invoke the runtime archive validator exactly once")
    if re.search(rf"(?m)^\s*{re.escape(validator_call)}\s+\\\s*$", body) is None:
        raise SystemExit(f"{step_name} must invoke the validator with python3 and continued arguments")
    has_fskit = "astrid-storage-provider-fskit" in body
    if step_name == "Compose the Darwin candidate" and not has_fskit:
        raise SystemExit("Darwin composition must require the FSKit provider")
    if step_name == "Compose the GNU native sealer source" and has_fskit:
        raise SystemExit("GNU composition must retain the four portable runtime binaries")

if re.search(r'(?m)^\s*"\$REHEARSAL_CHECKOUT/scripts/validate-runtime-archive\.py"', compose):
    raise SystemExit("compose job must not direct-exec the runtime archive validator")

lines = text.splitlines()
for index, line in enumerate(lines):
    if "scripts/validate-runtime-archive.py" in line and (
        re.search(r"\bchmod\b", line) or ".chmod(" in line
    ):
        raise SystemExit("rehearsal workflow must not chmod the runtime archive validator")
    if re.match(r"^\s*chmod\b", line):
        command = line
        end = index
        while command.rstrip().endswith("\\") and end + 1 < len(lines):
            end += 1
            command += "\n" + lines[end]
        if "scripts/validate-runtime-archive.py" in command:
            raise SystemExit("rehearsal workflow must not chmod the runtime archive validator")

if "Prove overlay-built GNU AOS accepts the signed Distro" not in compose:
    raise SystemExit("compose job must execute overlay-built AOS against the signed Distro")
probe = '"$LINUX_AOS_BINARY" distro apply --principal operator-qa --yes'
if probe not in compose:
    raise SystemExit("compose job must run overlay-built GNU AOS distro apply as the consume probe")
if 'tar -xzf "$SIGNED_GNU_ARCHIVE"' not in compose:
    raise SystemExit("GNU consume probe must consume the signed GNU archive")
if 'bundle="$extract/unicity-aos-${AOS_PRODUCT_VERSION}-${LINUX_TARGET}"' not in compose:
    raise SystemExit("GNU consume probe must select the signed GNU archive root")
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

for marker in (
    'cp "$SIGNED_DARWIN_ARCHIVE" "$output/"',
    'cp "$SIGNED_GNU_ARCHIVE" "$output/"',
    'b3sum -- "$darwin_asset"',
    'b3sum -- "$gnu_asset"',
    'sha256sum -- "$darwin_asset"',
    'sha256sum -- "$gnu_asset"',
    '"aarch64-apple-darwin": {',
    '"x86_64-unknown-linux-gnu": {',
    '"publication_allowed": False',
    '"key_scope": "ephemeral-per-run-qa"',
):
    if marker not in compose:
        raise SystemExit(f"rehearsal evidence is missing: {marker}")

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
