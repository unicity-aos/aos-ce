#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
python3 "$repo_root/scripts/validate-release-contract.py"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
target=x86_64-unknown-linux-gnu
read -r product_version runtime_version runtime_identity < <(
  python3 - "$repo_root" <<'PY'
import pathlib
import sys
import tomllib

root = pathlib.Path(sys.argv[1])
with (root / "crates/unicity-aos-bootstrap/Cargo.toml").open("rb") as file:
    product = tomllib.load(file)["package"]["version"]
with (root / "release/runtime-compatibility.toml").open("rb") as file:
    runtime = tomllib.load(file)["runtime"]
print(product, runtime["version"], runtime["release-workflow-identity"])
PY
)

# Release artifacts are downloaded by artifact name but signed by their
# product archive filename. Keep the workflow's discovery and fail-closed
# guards exercised here so either naming contract cannot silently drift.
sign_step=$(python3 - "$repo_root/.github/workflows/release.yml" <<'PY'
import pathlib
import sys

lines = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8").splitlines()
start = next(
    index
    for index, line in enumerate(lines)
    if line.strip() == "- name: Sign the selected Distro in each product archive"
)
end = next(
    index
    for index in range(start + 1, len(lines))
    if lines[index].startswith("      - name: ")
)
print("\n".join(lines[start:end]))
PY
)
if grep -Fq -- 'artifacts/aos-*-*-*.tar.gz' <<<"$sign_step"; then
  echo "release signing loop still uses artifact names instead of product archive names" >&2
  exit 1
fi
grep -Fq -- 'assets=(artifacts/unicity-aos-*.tar.gz)' <<<"$sign_step"
grep -Fq -- 'no product archives found to sign' <<<"$sign_step"
grep -Fq -- 'no product archives contain Distro.sig after signing' <<<"$sign_step"
native_sealer_ref="\"\$AOS_DISTRO_NATIVE_SEALER\""
native_sealer_archive_ref="\"\$AOS_DISTRO_NATIVE_SEALER_ARCHIVE\""
grep -Fq -- "$native_sealer_ref" <<<"$sign_step"
grep -Fq -- "$native_sealer_archive_ref" <<<"$sign_step"
grep -Fq -- 'pattern: aos-*-*-*' "$repo_root/.github/workflows/release.yml"
grep -Fq -- 'name: aos-x86_64-unknown-linux-gnu' "$repo_root/.github/workflows/release.yml"
grep -Fq -- '--extract-release-sealer' "$repo_root/.github/workflows/release.yml"
grep -Fq -- 'stable Distro sealing requires Astrid 2026.9.0+' "$repo_root/.github/workflows/release.yml"

assert_signed_product_archives() {
  local artifacts_dir=$1
  local signed_archives=0
  local -a assets

  shopt -s nullglob
  assets=("$artifacts_dir"/unicity-aos-*.tar.gz)
  (( ${#assets[@]} > 0 )) || return 1
  for asset in "${assets[@]}"; do
    if tar -tzf "$asset" 2>/dev/null | grep -q '/Distro.sig$'; then
      signed_archives=$((signed_archives + 1))
    fi
  done
  (( signed_archives > 0 ))
}

glob_work="$work/signing-glob"
mkdir -p "$glob_work/product" "$glob_work/signed" "$glob_work/empty" "$glob_work/wrong"
product_archive="$glob_work/product/unicity-aos-${product_version}-${target}.tar.gz"
printf 'product archive fixture\n' > "$product_archive"
shopt -s nullglob
old_assets=("$glob_work/product"/aos-*-*-*.tar.gz)
test "${#old_assets[@]}" -eq 0
new_assets=("$glob_work/product"/unicity-aos-*.tar.gz)
test "${#new_assets[@]}" -eq 1
test "$(basename "${new_assets[0]}")" = "unicity-aos-${product_version}-${target}.tar.gz"

printf 'fixture signature\n' > "$glob_work/signed/Distro.sig"
mkdir "$glob_work/signed/unicity-aos-${product_version}-${target}"
mv "$glob_work/signed/Distro.sig" \
  "$glob_work/signed/unicity-aos-${product_version}-${target}/Distro.sig"
COPYFILE_DISABLE=1 tar -czf \
  "$glob_work/signed/unicity-aos-${product_version}-${target}.tar.gz" \
  -C "$glob_work/signed" "unicity-aos-${product_version}-${target}"
assert_signed_product_archives "$glob_work/signed"
if assert_signed_product_archives "$glob_work/empty"; then
  echo "release signing guard accepted an empty artifact directory" >&2
  exit 1
fi
printf 'wrong archive name\n' > "$glob_work/wrong/aos-${product_version}-${target}-wrong.tar.gz"
if assert_signed_product_archives "$glob_work/wrong"; then
  echo "release signing guard accepted a directory without product archives" >&2
  exit 1
fi

runtime_root="$work/astrid-$runtime_version-$target"
mkdir -p "$runtime_root" "$work/output"
mkdir -p "$work/capsules"

PYTHONPATH="$repo_root/scripts" python3 - "$work/capsules" <<'PY'
import pathlib
import sys

from capsule_release import source_contract
from test_capsule_release import write_fixture

output = pathlib.Path(sys.argv[1])
for spec in source_contract():
    write_fixture(output / spec.asset, spec)
PY

for binary in astrid astrid-daemon astrid-build astrid-emit; do
  if [[ "$binary" == astrid ]]; then
    cat > "$runtime_root/$binary" <<'RUNTIME'
#!/bin/sh
set -eu
if [ -n "${AOS_DISTRO_ED25519_SEED:-}" ]; then
  exit 96
fi
if [ "${1:-}" = --version ]; then
  echo "astrid __RUNTIME_VERSION__"
  exit 0
fi
if [ "${2:-}" != seal ]; then exit 0; fi
manifest=$3
output=
key=
while [ "$#" -gt 0 ]; do
  case "$1" in
    --output) output=$2; shift 2 ;;
    --key) key=$2; shift 2 ;;
    *) shift ;;
  esac
done
[ -n "$output" ] && [ -f "$key" ]
stage=$(dirname "$output")/aos-fixture-seal
rm -rf "$stage"
mkdir -p "$stage/capsules"
cp "$manifest" "$stage/Distro.toml"
cp "$(dirname "$manifest")"/capsules/*.capsule "$stage/capsules/"
printf 'schema-version = 1\nfixture = true\n' > "$stage/Distro.lock"
printf 'fixture-signature\n' > "$stage/Distro.sig"
printf '  signed by ed25519:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=\n' >&2
COPYFILE_DISABLE=1 tar -czf "$output" -C "$stage" Distro.toml Distro.lock Distro.sig capsules
RUNTIME
  else
    printf '#!/bin/sh\nexit 0\n' > "$runtime_root/$binary"
  fi
  chmod 755 "$runtime_root/$binary"
done
sed -i.bak "s/__RUNTIME_VERSION__/$runtime_version/" "$runtime_root/astrid"
rm "$runtime_root/astrid.bak"
COPYFILE_DISABLE=1 tar -czf "$work/runtime.tar.gz" -C "$work" "$(basename "$runtime_root")"
printf '#!/bin/sh\nexit 0\n' > "$work/aos"
chmod 755 "$work/aos"

if bash "$repo_root/scripts/package-release.sh" \
  "$target" \
  "$work/aos" \
  "$work/runtime.tar.gz" \
  AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA \
  "$work/capsules" \
  "$work/output" >/dev/null 2>&1; then
  echo "release composer accepted a non-canonical BLAKE3 digest" >&2
  exit 1
fi

bash "$repo_root/scripts/package-release.sh" \
  "$target" \
  "$work/aos" \
  "$work/runtime.tar.gz" \
  0000000000000000000000000000000000000000000000000000000000000000 \
  "$work/capsules" \
  "$work/output"

archive="$work/output/unicity-aos-$product_version-$target.tar.gz"
test -f "$archive"
tar -tzf "$archive" > "$work/files"
grep -q '/bin/aos$' "$work/files"
grep -q '/libexec/install.sh$' "$work/files"
grep -q '/runtime/bin/astrid-daemon$' "$work/files"
grep -q '/runtime-compatibility.toml$' "$work/files"
test "$(grep -c '/capsules/aos-.*\.capsule$' "$work/files")" -eq 22
grep -q '/capsule-assets.txt$' "$work/files"
grep -q '/Distro.toml$' "$work/files"
grep -q '/release-manifest.json$' "$work/files"
if grep -Eq '/Distro\.(lock|sig)$' "$work/files"; then
  echo "unsigned release packaging emitted a production Distro signature" >&2
  exit 1
fi

tar -xzf "$archive" -C "$work"
manifest=$(find "$work" -path '*/release-manifest.json' -print -quit)
bundle_root=$(dirname "$manifest")
test "$(stat -c '%a' "$bundle_root/bin/aos" 2>/dev/null || stat -f '%Lp' "$bundle_root/bin/aos")" = 755
test "$(stat -c '%a' "$bundle_root/runtime/bin/astrid-daemon" 2>/dev/null || stat -f '%Lp' "$bundle_root/runtime/bin/astrid-daemon")" = 755
test "$(grep -c '^source = "capsules/aos-.*\.capsule"$' "$bundle_root/Distro.toml")" -eq 22
if grep -F '@unicity-aos/capsule-' "$bundle_root/Distro.toml" >/dev/null; then
  echo "release archive retained a legacy capsule repository source" >&2
  exit 1
fi
python3 - "$manifest" "$product_version" "$runtime_version" "$runtime_identity" <<'PY'
import json
import pathlib
import sys

manifest = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
product_version, runtime_version, runtime_identity = sys.argv[2:]
assert manifest["schema_version"] == 2
assert manifest["product"]["version"] == product_version
assert manifest["layout"] == {
    "release_directory": f"releases/{product_version}",
    "runtime_executables": "runtime/bin",
    "capsule_assets": "capsules",
}
expected_inventory = {
    "Distro.toml",
    "README.md",
    "bin/aos",
    "capsule-assets.txt",
    "libexec/install.sh",
    "runtime-compatibility.toml",
    "runtime/bin/astrid",
    "runtime/bin/astrid-build",
    "runtime/bin/astrid-daemon",
    "runtime/bin/astrid-emit",
    *(f"capsules/{asset}" for asset in manifest["capsules"]["assets"]),
}
assert set(manifest["release_files"]) == expected_inventory, (
    set(manifest["release_files"]) - expected_inventory,
    expected_inventory - set(manifest["release_files"]),
)
assert manifest["release_files"]["bin/aos"]["mode"] == 0o755
assert manifest["release_files"]["libexec/install.sh"]["mode"] == 0o600
assert manifest["release_files"]["runtime/bin/astrid-daemon"]["mode"] == 0o755
assert manifest["runtime"]["version"] == runtime_version
assert manifest["runtime"]["digest"] == "blake3:" + "0" * 64
assert "sha256" not in manifest["runtime"]
assert manifest["runtime"]["release_workflow_identity"] == runtime_identity
assert manifest["contracts"]["repository"] == "astrid-runtime/wit"
assert manifest["contracts"]["commit"] == "278dbca3e32f327d0f2358644fc86559779ba0fd"
assert manifest["contracts"]["sdk_rust_version"] == "0.7.1"
assert manifest["contracts"]["sdk_rust_commit"] == "bbbc61c8821d6c536fb25d2068b6b646e759ad35"
assert manifest["capsules"]["count"] == 22
assert len(manifest["capsules"]["assets"]) == 22
assert len(set(manifest["capsules"]["assets"])) == 22
assert manifest["verifier"] == {
    "version": "v3.1.1",
    "asset": "cosign-linux-amd64",
    "sha256": "ae1ecd212663f3693ad9edf8b1a183900c9a52d3155ba6e354237f9a0f6463fc",
}
PY

fixture_distro="$work/FixtureDistro.toml"
python3 - "$bundle_root/Distro.toml" "$fixture_distro" <<'PY'
import pathlib
import sys

source = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8")
official = 'pubkey = "ed25519:utH537RuOuqKwjGx/pHIUAkKapyqPUhHpZIVDU6Q0FA="'
fixture = 'pubkey = "ed25519:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="'
if source.count(official) != 1:
    raise SystemExit("product Distro does not carry exactly one official signing pin")
pathlib.Path(sys.argv[2]).write_text(source.replace(official, fixture), encoding="utf-8")
PY
cp "$fixture_distro" "$bundle_root/Distro.toml"
fixture_archive="$work/fixture-aos.tar.gz"
COPYFILE_DISABLE=1 tar -czf "$fixture_archive" -C "$work" "$(basename "$bundle_root")"
fixture_signed="$work/fixture-aos-signed.tar.gz"
native_sealer="$work/native-distro-sealer"
bash "$repo_root/scripts/package-release.sh" \
  --extract-release-sealer "$fixture_archive" "$native_sealer"
AOS_DISTRO_ED25519_SEED="$(python3 -c 'import secrets; print(secrets.token_hex(32))')" \
  bash "$repo_root/scripts/package-release.sh" \
  --sign-release-archive "$fixture_archive" "$fixture_signed" "$native_sealer" "$fixture_archive"
unset AOS_DISTRO_ED25519_SEED
tar -tzf "$fixture_signed" > "$work/fixture-files"
grep -q '/Distro.lock$' "$work/fixture-files"
grep -q '/Distro.sig$' "$work/fixture-files"
mkdir "$work/fixture-extract"
tar -xzf "$fixture_signed" -C "$work/fixture-extract"
fixture_root="$work/fixture-extract/$(basename "$bundle_root")"
cmp "$fixture_distro" "$fixture_root/Distro.toml"
python3 - "$fixture_root/release-manifest.json" <<'PY'
import json
import pathlib
import sys

files = json.loads(pathlib.Path(sys.argv[1]).read_text())["release_files"]
assert {"Distro.lock", "Distro.sig"} <= files.keys()
PY

python3 - "$bundle_root/Distro.toml" "$fixture_distro" <<'PY'
import pathlib
import sys

source = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8")
fixture = 'pubkey = "ed25519:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="'
mismatch = 'pubkey = "ed25519:BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB="'
if source.count(fixture) != 1:
    raise SystemExit("fixture Distro does not carry exactly one fixture signing pin")
pathlib.Path(sys.argv[2]).write_text(source.replace(fixture, mismatch), encoding="utf-8")
PY
cp "$fixture_distro" "$bundle_root/Distro.toml"
mismatch_archive="$work/mismatch-aos.tar.gz"
COPYFILE_DISABLE=1 tar -czf "$mismatch_archive" -C "$work" "$(basename "$bundle_root")"
if AOS_DISTRO_ED25519_SEED="$(python3 -c 'import secrets; print(secrets.token_hex(32))')" \
  bash "$repo_root/scripts/package-release.sh" \
  --sign-release-archive "$mismatch_archive" "$work/mismatch-aos-signed.tar.gz" "$native_sealer" "$fixture_archive" >/dev/null 2>&1; then
  echo "release signing accepted a sealer key that differs from Distro.toml" >&2
  exit 1
fi

multi_target_root="$work/multi-target-archives"
mkdir "$multi_target_root"
targets=(
  x86_64-unknown-linux-gnu
  aarch64-unknown-linux-gnu
  x86_64-apple-darwin
  aarch64-apple-darwin
)
for archive_target in "${targets[@]}"; do
  target_root="$multi_target_root/unicity-aos-${product_version}-${archive_target}"
  cp -R "$bundle_root" "$target_root"
  cp "$fixture_root/Distro.toml" "$target_root/Distro.toml"
  rm -f "$target_root/Distro.lock" "$target_root/Distro.sig"
  cat > "$target_root/runtime/bin/astrid" <<'SENTINEL'
#!/bin/sh
touch "${AOS_TEST_SENTINEL:?}"
exit 97
SENTINEL
  chmod 755 "$target_root/runtime/bin/astrid"
  sentinel_digest=$(b3sum -- "$target_root/runtime/bin/astrid" | awk '{print $1}')
  python3 - "$target_root/release-manifest.json" "$archive_target" "$runtime_version" "$sentinel_digest" <<'PY'
import json
import pathlib
import sys

path, target, runtime_version, digest = sys.argv[1:]
manifest = json.loads(pathlib.Path(path).read_text(encoding="utf-8"))
manifest["target"] = target
manifest["runtime"]["asset"] = f"astrid-{runtime_version}-{target}.tar.gz"
manifest["release_files"]["runtime/bin/astrid"] = {"blake3": digest, "mode": 0o755}
pathlib.Path(path).write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
PY
  COPYFILE_DISABLE=1 tar -czf \
    "$multi_target_root/unicity-aos-${product_version}-${archive_target}.tar.gz" \
    -C "$multi_target_root" "$(basename "$target_root")"
  rm -rf "$target_root"
done

sentinel="$work/embedded-target-sealer-invoked"
for unsigned_archive in "$multi_target_root"/*.tar.gz; do
  signed_archive="$work/signed-$(basename "$unsigned_archive")"
  AOS_TEST_SENTINEL="$sentinel" \
    AOS_DISTRO_ED25519_SEED="$(python3 -c 'import secrets; print(secrets.token_hex(32))')" \
    bash "$repo_root/scripts/package-release.sh" \
    --sign-release-archive "$unsigned_archive" "$signed_archive" "$native_sealer" "$fixture_archive" >/dev/null
  test ! -e "$sentinel"
  tar -tzf "$signed_archive" | grep -q '/Distro.lock$'
  tar -tzf "$signed_archive" | grep -q '/Distro.sig$'
  mkdir "$work/signed-$(basename "$unsigned_archive" .tar.gz)"
  tar -xzf "$signed_archive" -C "$work/signed-$(basename "$unsigned_archive" .tar.gz)"
  signed_root=$(find "$work/signed-$(basename "$unsigned_archive" .tar.gz)" -mindepth 1 -maxdepth 1 -type d -print -quit)
  cmp "$fixture_root/Distro.toml" "$signed_root/Distro.toml"
done

wrong_target_source="$work/wrong-target-sealer-source.tar.gz"
wrong_target_root="$work/unicity-aos-${product_version}-x86_64-unknown-linux-gnu-wrong-target"
cp -R "$fixture_root" "$wrong_target_root"
python3 - "$wrong_target_root/release-manifest.json" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
manifest = json.loads(path.read_text(encoding="utf-8"))
manifest["target"] = "aarch64-unknown-linux-gnu"
path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
PY
COPYFILE_DISABLE=1 tar -czf "$wrong_target_source" -C "$work" "$(basename "$wrong_target_root")"
if bash "$repo_root/scripts/package-release.sh" \
  --extract-release-sealer "$wrong_target_source" "$work/wrong-target-sealer" >/dev/null 2>&1; then
  echo "release sealer extraction accepted a source archive with the wrong target" >&2
  exit 1
fi

wrong_digest_source="$work/wrong-digest-sealer-source.tar.gz"
wrong_digest_root="$work/unicity-aos-${product_version}-x86_64-unknown-linux-gnu-wrong-digest"
cp -R "$fixture_root" "$wrong_digest_root"
python3 - "$wrong_digest_root/release-manifest.json" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
manifest = json.loads(path.read_text(encoding="utf-8"))
manifest["release_files"]["runtime/bin/astrid"]["blake3"] = "f" * 64
path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
PY
COPYFILE_DISABLE=1 tar -czf "$wrong_digest_source" -C "$work" "$(basename "$wrong_digest_root")"
if bash "$repo_root/scripts/package-release.sh" \
  --extract-release-sealer "$wrong_digest_source" "$work/wrong-digest-sealer" >/dev/null 2>&1; then
  echo "release sealer extraction accepted bytes that differ from the manifest" >&2
  exit 1
fi

wrong_version_source="$work/wrong-version-sealer-source.tar.gz"
wrong_version_root="$work/unicity-aos-${product_version}-x86_64-unknown-linux-gnu-wrong-version"
cp -R "$fixture_root" "$wrong_version_root"
sed -i.bak "s/astrid $runtime_version/astrid 9.9.9/" "$wrong_version_root/runtime/bin/astrid"
rm "$wrong_version_root/runtime/bin/astrid.bak"
wrong_version_digest=$(b3sum -- "$wrong_version_root/runtime/bin/astrid" | awk '{print $1}')
python3 - "$wrong_version_root/release-manifest.json" "$wrong_version_digest" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
manifest = json.loads(path.read_text(encoding="utf-8"))
manifest["release_files"]["runtime/bin/astrid"]["blake3"] = sys.argv[2]
path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
PY
COPYFILE_DISABLE=1 tar -czf "$wrong_version_source" -C "$work" "$(basename "$wrong_version_root")"
if bash "$repo_root/scripts/package-release.sh" \
  --extract-release-sealer "$wrong_version_source" "$work/wrong-version-sealer" >/dev/null 2>&1; then
  echo "release sealer extraction accepted a binary with the wrong version" >&2
  exit 1
fi

if AOS_DISTRO_ED25519_SEED="$(python3 -c 'import secrets; print(secrets.token_hex(32))')" \
  bash "$repo_root/scripts/package-release.sh" \
  --sign-release-archive "$fixture_archive" "$work/missing-sealer-signed.tar.gz" "$work/missing-sealer" "$fixture_archive" >/dev/null 2>&1; then
  echo "release signing accepted a missing explicit native sealer" >&2
  exit 1
fi

unsafe_root="$work/unsafe-runtime"
mkdir -p "$unsafe_root/astrid-$runtime_version-$target"
ln -s /tmp "$unsafe_root/astrid-$runtime_version-$target/astrid"
for binary in astrid-daemon astrid-build astrid-emit; do
  printf '#!/bin/sh\nexit 0\n' > "$unsafe_root/astrid-$runtime_version-$target/$binary"
  chmod 755 "$unsafe_root/astrid-$runtime_version-$target/$binary"
done
COPYFILE_DISABLE=1 tar -czf "$work/unsafe-runtime.tar.gz" -C "$unsafe_root" "astrid-$runtime_version-$target"
if bash "$repo_root/scripts/package-release.sh" \
  "$target" \
  "$work/aos" \
  "$work/unsafe-runtime.tar.gz" \
  0000000000000000000000000000000000000000000000000000000000000000 \
  "$work/capsules" \
  "$work/output" >/dev/null 2>&1; then
  echo "release composer accepted a symlinked runtime binary" >&2
  exit 1
fi

python3 - "$work/duplicate-runtime.tar.gz" "$target" "$runtime_version" <<'PY'
import io
import sys
import tarfile

archive_path, target, runtime_version = sys.argv[1:]
root = f"astrid-{runtime_version}-{target}"

def add(archive, name, data=b"#!/bin/sh\nexit 0\n"):
    member = tarfile.TarInfo(name)
    member.mode = 0o755
    member.size = len(data)
    archive.addfile(member, io.BytesIO(data))

with tarfile.open(archive_path, "w:gz") as archive:
    directory = tarfile.TarInfo(root)
    directory.type = tarfile.DIRTYPE
    directory.mode = 0o755
    archive.addfile(directory)
    for binary in ("astrid", "astrid-daemon", "astrid-build", "astrid-emit"):
        add(archive, f"{root}/{binary}")
    add(archive, f"{root}/astrid", b"#!/bin/sh\nexit 99\n")
PY

if bash "$repo_root/scripts/package-release.sh" \
  "$target" \
  "$work/aos" \
  "$work/duplicate-runtime.tar.gz" \
  0000000000000000000000000000000000000000000000000000000000000000 \
  "$work/capsules" \
  "$work/output" >/dev/null 2>&1; then
  echo "release composer accepted a duplicate runtime binary" >&2
  exit 1
fi

rm "$work/capsules/aos-cli.capsule"
if bash "$repo_root/scripts/package-release.sh" \
  "$target" \
  "$work/aos" \
  "$work/runtime.tar.gz" \
  0000000000000000000000000000000000000000000000000000000000000000 \
  "$work/capsules" \
  "$work/output" >/dev/null 2>&1; then
  echo "release composer accepted an incomplete capsule set" >&2
  exit 1
fi
