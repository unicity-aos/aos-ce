#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
fixture="$work/fixture"
fake_bin="$work/fake-bin"
mkdir -p "$fixture" "$fake_bin" "$work/home" "$work/capsules"
mkdir -p "$work/home/.astrid"
printf 'standalone-runtime-state\n' > "$work/home/.astrid/sentinel"
if ! read -r \
  runtime_version \
  runtime_tag \
  runtime_identity \
  runtime_metadata_available \
  runtime_source_commit \
  runtime_metadata_asset \
  runtime_metadata_blake3 < <(
  python3 - "$repo_root/release/runtime-compatibility.toml" <<'PY'
import pathlib
import sys
import tomllib

with pathlib.Path(sys.argv[1]).open("rb") as file:
    runtime = tomllib.load(file)["runtime"]
print(
    runtime["version"],
    runtime["tag"],
    runtime["release-workflow-identity"],
    str(runtime["release-metadata-available"]).lower(),
    runtime["source-commit"],
    runtime["release-metadata-asset"],
    runtime["release-metadata-blake3"],
)
PY
); then
  echo "failed to read runtime compatibility fixture provenance" >&2
  exit 1
fi
if [[ -z "$runtime_version" || -z "$runtime_tag" || -z "$runtime_identity" || \
      -z "$runtime_source_commit" || -z "$runtime_metadata_asset" || \
      -z "$runtime_metadata_blake3" ]]; then
  echo "runtime compatibility fixture provenance is incomplete" >&2
  exit 1
fi
if [[ "$runtime_metadata_available" != true ]]; then
  echo "installer fixture must exercise available runtime release metadata" >&2
  exit 1
fi

cat > "$work/aos" <<'EOF'
#!/bin/sh
if [ "${1:-}" = --version ]; then
  echo 'Unicity AOS 2026.9.0'
  exit 0
fi
exit 0
EOF
chmod 755 "$work/aos"

PYTHONPATH="$repo_root/scripts" python3 - "$work/capsules" <<'PY'
import pathlib
import sys

from capsule_release import source_contract
from test_capsule_release import write_fixture

output = pathlib.Path(sys.argv[1])
for spec in source_contract():
    write_fixture(output / spec.asset, spec)
PY

runtime_root="$work/astrid-$runtime_version-x86_64-unknown-linux-gnu"
mkdir -p "$runtime_root"
for binary in astrid astrid-daemon astrid-build astrid-emit; do
  printf '#!/bin/sh\necho %s\n' "$binary" > "$runtime_root/$binary"
  chmod 755 "$runtime_root/$binary"
done
COPYFILE_DISABLE=1 tar -czf "$work/runtime.tar.gz" -C "$work" "$(basename "$runtime_root")"
bash "$repo_root/scripts/package-release.sh" \
  x86_64-unknown-linux-gnu \
  "$work/aos" \
  "$work/runtime.tar.gz" \
  0000000000000000000000000000000000000000000000000000000000000000 \
  "$work/capsules" \
  "$fixture" >/dev/null
asset="$fixture/unicity-aos-2026.9.0-x86_64-unknown-linux-gnu.tar.gz"
bundle="$asset.sigstore.json"
signed_asset="$fixture/signed-asset.tar.gz"
good_bundle="$fixture/valid.sigstore.json"
cp "$asset" "$signed_asset"
printf 'valid Sigstore fixture\n' > "$good_bundle"
cp "$good_bundle" "$bundle"

asset_sha256=$(shasum -a 256 "$asset" | awk '{print $1}')
asset_blake3=$(b3sum "$asset" | awk '{print $1}')
asset_size=$(wc -c < "$asset" | tr -d ' ')
release_metadata="$fixture/unicity-aos-2026.9.0-release.toml"
cat > "$release_metadata" <<EOF
schema-version = 1
kind = "aos-release"
product = "unicity-aos-ce"
version = "2026.9.0"
tag = "2026.9.0"
source-commit = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
published-at = "2026-07-16T10:00:00Z"
release-workflow-identity = "https://github.com/unicity-aos/aos-ce/.github/workflows/release.yml@refs/tags/2026.9.0"

[runtime]
repository = "astrid-runtime/astrid"
version = "${runtime_version}"
tag = "${runtime_tag}"
release-workflow-identity = "${runtime_identity}"
release-metadata-available = ${runtime_metadata_available}
source-commit = "${runtime_source_commit}"
release-metadata-asset = "${runtime_metadata_asset}"
release-metadata-blake3 = "${runtime_metadata_blake3}"

[contracts]
repository = "astrid-runtime/wit"
commit = "278dbca3e32f327d0f2358644fc86559779ba0fd"
sdk-rust-version = "0.7.1"
sdk-rust-commit = "bbbc61c8821d6c536fb25d2068b6b646e759ad35"

[gates]
release-ready = true
upgrade-self-heal-ready = true
EOF
for metadata_target in aarch64-apple-darwin x86_64-apple-darwin aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu; do
  metadata_asset="unicity-aos-2026.9.0-${metadata_target}.tar.gz"
  cat >> "$release_metadata" <<EOF

[targets.${metadata_target}]
asset = "${metadata_asset}"
sha256 = "${asset_sha256}"
blake3 = "${asset_blake3}"
sigstore-bundle = "${metadata_asset}.sigstore.json"
size = ${asset_size}
EOF
done
cp "$good_bundle" "$release_metadata.sigstore.json"
cp "$release_metadata" "$fixture/release-good.toml"

cat > "$fake_bin/uname" <<'EOF'
#!/bin/sh
case "${1:-}" in
  -s) echo "${AOS_TEST_UNAME_S:-Linux}" ;;
  -m) echo "${AOS_TEST_UNAME_M:-x86_64}" ;;
  *) exit 2 ;;
esac
EOF
cat > "$fake_bin/date" <<'EOF'
#!/bin/sh
if [ "$#" -eq 2 ] && [ "$1" = -u ]; then
  case "$2" in
    +%Y-%m-%dT%H:%M:%SZ) printf '%s\n' '2026-07-16T10:00:00Z'; exit 0 ;;
    +%s) printf '%s\n' '1784196000'; exit 0 ;;
  esac
fi
exec /bin/date "$@"
EOF

cat > "$fake_bin/curl" <<'EOF'
#!/bin/sh
set -eu
output=
url=
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o) output=$2; shift ;;
    http*) url=$1 ;;
  esac
  shift
done
[ -n "$output" ]
[ -n "$url" ]
cp "$AOS_TEST_FIXTURE/$(basename "$url")" "$output"
EOF
cat > "$fixture/cosign-linux-amd64" <<'EOF'
#!/bin/sh
set -eu
[ "${1:-}" = verify-blob ]
bundle=
artifact=
identity=
while [ "$#" -gt 0 ]; do
  case "$1" in
    --bundle)
      bundle=$2
      shift
      ;;
    --certificate-identity)
      identity=$2
      shift
      ;;
    --certificate-identity-regexp) exit 97 ;;
    -*) ;;
    *) artifact=$1 ;;
  esac
  shift
done
[ -n "$bundle" ]
[ -n "$artifact" ]
[ -n "$identity" ]
cmp "$AOS_TEST_FIXTURE/valid.sigstore.json" "$bundle"
[ -f "$artifact" ]
printf '%s\n' "$identity" >> "$AOS_TEST_FIXTURE/cosign-identities"
: > "$AOS_TEST_FIXTURE/cosign-called"
EOF
cat > "$fake_bin/cosign" <<'EOF'
#!/bin/sh
set -eu
: > "$AOS_TEST_FIXTURE/path-cosign-called"
exit 99
EOF

cat > "$fake_bin/sha256sum" <<'EOF'
#!/bin/sh
set -eu
case "$1" in
  */cosign)
    if [ "${AOS_TEST_BAD_COSIGN_DIGEST:-0}" = 1 ]; then
      printf '%064d  %s\n' 0 "$1"
    else
      printf '%s  %s\n' \
        "${AOS_TEST_COSIGN_SHA256:-ae1ecd212663f3693ad9edf8b1a183900c9a52d3155ba6e354237f9a0f6463fc}" \
        "$1"
    fi
    ;;
  *) exec /usr/bin/shasum -a 256 "$1" ;;
esac
EOF
chmod 755 "$fake_bin/uname" "$fake_bin/date" "$fake_bin/curl" "$fake_bin/cosign" \
  "$fake_bin/sha256sum" "$fixture/cosign-linux-amd64"

if PATH="$fake_bin:$PATH" HOME="$work/impossible-nightly-home" AOS_TEST_FIXTURE="$fixture" \
  sh "$repo_root/install.sh" --version "2026.9.0-nightly.20260230.g$(printf '%040d' 0)" --yes --no-migrate-prompt >/dev/null 2>&1; then
  echo "installer accepted a nightly version with an impossible date" >&2
  exit 1
fi
test ! -e "$work/impossible-nightly-home/.aos"

PATH="$fake_bin:$PATH" \
HOME="$work/home" \
AOS_TEST_FIXTURE="$fixture" \
AOS_VERSION=2026.9.0 \
sh "$repo_root/install.sh" --yes --no-migrate-prompt

test -x "$work/home/.aos/bin/aos"
release_dir="$work/home/.aos/releases/2026.9.0"
test "$runtime_version" = 0.10.4
for binary in astrid astrid-daemon astrid-build astrid-emit; do
  test -x "$release_dir/runtime/bin/$binary"
  test ! -e "$work/home/.aos/runtime/bin/$binary"
done
test ! -e "$release_dir/runtime/bin/astrid-storage-provider-fuse"
test -f "$release_dir/release-manifest.json"
test -f "$release_dir/Distro.toml"
test -f "$release_dir/capsule-assets.txt"
test "$(find "$release_dir/capsules" -mindepth 1 -maxdepth 1 -type f | wc -l | tr -d ' ')" -eq 22
while IFS= read -r capsule; do
  cmp "$work/capsules/$capsule" "$release_dir/capsules/$capsule"
done < "$release_dir/capsule-assets.txt"
test "$("$work/home/.aos/bin/aos" --version)" = 'Unicity AOS 2026.9.0'
test -f "$work/home/.aos/libexec/install.sh"
test "$(stat -c '%a' "$work/home/.aos/libexec/install.sh" 2>/dev/null || stat -f '%Lp' "$work/home/.aos/libexec/install.sh")" = 600
test "$(cat "$work/home/.astrid/sentinel")" = 'standalone-runtime-state'
test -f "$fixture/cosign-called"
test ! -e "$fixture/path-cosign-called"
test "$(stat -c '%a' "$work/home/.aos" 2>/dev/null || stat -f '%Lp' "$work/home/.aos")" = 700
test "$(stat -c '%a' "$release_dir/release-manifest.json" 2>/dev/null || stat -f '%Lp' "$release_dir/release-manifest.json")" = 600
test "$(stat -c '%a' "$release_dir/runtime/bin/astrid" 2>/dev/null || stat -f '%Lp' "$release_dir/runtime/bin/astrid")" = 700
test "$(stat -c '%a' "$release_dir/capsules" 2>/dev/null || stat -f '%Lp' "$release_dir/capsules")" = 700

darwin_fixture="$work/darwin-fixture"
darwin_runtime_root="$work/astrid-$runtime_version-aarch64-apple-darwin"
darwin_home="$work/darwin-home"
mkdir "$darwin_fixture" "$darwin_runtime_root" "$darwin_home"
for binary in \
  astrid astrid-daemon astrid-build astrid-emit \
  astrid-storage-provider-fskit
do
  source_binary=$binary
  if [[ "$binary" == astrid-storage-provider-fskit ]]; then
    source_binary=astrid
  fi
  cp "$runtime_root/$source_binary" "$darwin_runtime_root/$binary"
  chmod 755 "$darwin_runtime_root/$binary"
done
COPYFILE_DISABLE=1 tar -czf "$work/darwin-runtime.tar.gz" \
  -C "$work" "$(basename "$darwin_runtime_root")"
bash "$repo_root/scripts/package-release.sh" \
  aarch64-apple-darwin \
  "$work/aos" \
  "$work/darwin-runtime.tar.gz" \
  0000000000000000000000000000000000000000000000000000000000000000 \
  "$work/capsules" \
  "$darwin_fixture" >/dev/null
darwin_asset="$darwin_fixture/unicity-aos-2026.9.0-aarch64-apple-darwin.tar.gz"
darwin_sha256=$(shasum -a 256 "$darwin_asset" | awk '{print $1}')
darwin_blake3=$(b3sum "$darwin_asset" | awk '{print $1}')
darwin_size=$(wc -c < "$darwin_asset" | tr -d ' ')
python3 - \
  "$release_metadata" \
  "$darwin_fixture/unicity-aos-2026.9.0-release.toml" \
  "$darwin_sha256" \
  "$darwin_blake3" \
  "$darwin_size" <<'PY'
import pathlib
import sys

source, destination, sha256, blake3, size = sys.argv[1:]
lines = pathlib.Path(source).read_text(encoding="utf-8").splitlines()
inside = False
for index, line in enumerate(lines):
    if line.startswith("["):
        inside = line == "[targets.aarch64-apple-darwin]"
    if not inside:
        continue
    if line.startswith("sha256 = "):
        lines[index] = f'sha256 = "{sha256}"'
    elif line.startswith("blake3 = "):
        lines[index] = f'blake3 = "{blake3}"'
    elif line.startswith("size = "):
        lines[index] = f"size = {size}"
pathlib.Path(destination).write_text("\n".join(lines) + "\n", encoding="utf-8")
PY
cp "$good_bundle" "$darwin_fixture/unicity-aos-2026.9.0-release.toml.sigstore.json"
cp "$good_bundle" "$darwin_asset.sigstore.json"
cp "$good_bundle" "$darwin_fixture/valid.sigstore.json"
cp "$fixture/cosign-linux-amd64" "$darwin_fixture/cosign-darwin-arm64"
chmod 755 "$darwin_fixture/cosign-darwin-arm64"
PATH="$fake_bin:$PATH" \
HOME="$darwin_home" \
AOS_TEST_FIXTURE="$darwin_fixture" \
AOS_TEST_UNAME_S=Darwin \
AOS_TEST_UNAME_M=arm64 \
AOS_TEST_COSIGN_SHA256=94b42a9e697be95675f6160ab031a9a5f1ec1e646d6f648d7b2f5cd59ececbc5 \
AOS_VERSION=2026.9.0 \
sh "$repo_root/install.sh" --yes --no-migrate-prompt >/dev/null
darwin_release_dir="$darwin_home/.aos/releases/2026.9.0"
for binary in \
  astrid astrid-daemon astrid-build astrid-emit \
  astrid-storage-provider-fskit
do
  test -x "$darwin_release_dir/runtime/bin/$binary"
  test ! -e "$darwin_home/.aos/runtime/bin/$binary"
done

# Build a second package through the real composer with an isolated
# compatibility overlay.  The checked-in 0.10.4 contract above remains the
# historical control; this fixture exercises the versioned 2026.9.0 GNU
# membership that requires the FUSE provider.
fuse_repo="$work/aos-2026.9.0-contract"
mkdir -p \
  "$fuse_repo/scripts" \
  "$fuse_repo/crates/unicity-aos-bootstrap" \
  "$fuse_repo/distros/community/unicity-ce"
cp "$repo_root/Cargo.toml" "$fuse_repo/Cargo.toml"
cp "$repo_root/crates/unicity-aos-bootstrap/Cargo.toml" \
  "$fuse_repo/crates/unicity-aos-bootstrap/Cargo.toml"
cp -R "$repo_root/capsules" "$fuse_repo/"
cp -R "$repo_root/release" "$fuse_repo/"
cp "$repo_root/distros/community/unicity-ce/Distro.toml" \
  "$fuse_repo/distros/community/unicity-ce/Distro.toml"
cp "$repo_root/install.sh" "$repo_root/README.md" "$fuse_repo/"
cp "$repo_root/scripts/capsule_release.py" \
  "$repo_root/scripts/package-release.sh" \
  "$repo_root/scripts/validate-runtime-archive.py" \
  "$fuse_repo/scripts/"
python3 - "$fuse_repo/release/runtime-compatibility.toml" \
  "$fuse_repo/distros/community/unicity-ce/Distro.toml" <<'PY'
import pathlib
import sys

runtime_path, distro_path = map(pathlib.Path, sys.argv[1:])
runtime_lines = runtime_path.read_text(encoding="utf-8").splitlines()
replacements = {
    "version": 'version = "2026.9.0"',
    "tag": 'tag = "v2026.9.0"',
    "version-requirement": 'version-requirement = "=2026.9.0"',
    "release-workflow-identity": 'release-workflow-identity = "https://github.com/astrid-runtime/astrid/.github/workflows/release.yml@refs/tags/v2026.9.0"',
    "source-commit": 'source-commit = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"',
    "release-metadata-asset": 'release-metadata-asset = "astrid-2026.9.0-release.toml"',
    "release-metadata-blake3": 'release-metadata-blake3 = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"',
}
in_runtime = False
for index, line in enumerate(runtime_lines):
    if line == "[runtime]":
        in_runtime = True
        continue
    if line.startswith("["):
        in_runtime = False
    if in_runtime and "=" in line:
        key = line.split("=", 1)[0].strip()
        if key in replacements:
            runtime_lines[index] = replacements[key]
runtime_path.write_text("\n".join(runtime_lines) + "\n", encoding="utf-8")
distro_text = distro_path.read_text(encoding="utf-8")
distro_path.write_text(
    distro_text.replace('astrid-version = "=0.10.4"', 'astrid-version = "=2026.9.0"'),
    encoding="utf-8",
)
PY

fuse_runtime_root="$work/astrid-2026.9.0-x86_64-unknown-linux-gnu"
fuse_runtime_archive="$work/runtime-2026.9.0.tar.gz"
fuse_output="$work/output-2026.9.0"
mkdir -p "$fuse_runtime_root" "$fuse_output"
for binary in \
  astrid astrid-daemon astrid-build astrid-emit \
  astrid-storage-provider-fuse
do
  printf '#!/bin/sh\necho packaged-%s\n' "$binary" > "$fuse_runtime_root/$binary"
  chmod 755 "$fuse_runtime_root/$binary"
done
COPYFILE_DISABLE=1 tar -czf "$fuse_runtime_archive" \
  -C "$work" "$(basename "$fuse_runtime_root")"
bash "$fuse_repo/scripts/package-release.sh" \
  x86_64-unknown-linux-gnu \
  "$work/aos" \
  "$fuse_runtime_archive" \
  0000000000000000000000000000000000000000000000000000000000000000 \
  "$work/capsules" \
  "$fuse_output" >/dev/null
fuse_asset_name=unicity-aos-2026.9.0-x86_64-unknown-linux-gnu.tar.gz
fuse_asset="$fuse_output/$fuse_asset_name"
fuse_fixture="$work/fuse-fixture"
mkdir -p "$fuse_fixture"
cp "$fuse_asset" "$fuse_fixture/$fuse_asset_name"
fuse_asset_sha256=$(shasum -a 256 "$fuse_asset" | awk '{print $1}')
fuse_asset_blake3=$(b3sum "$fuse_asset" | awk '{print $1}')
fuse_asset_size=$(wc -c < "$fuse_asset" | tr -d ' ')
fuse_release_metadata="$fuse_fixture/unicity-aos-2026.9.0-release.toml"
cat > "$fuse_release_metadata" <<EOF
schema-version = 1
kind = "aos-release"
product = "unicity-aos-ce"
version = "2026.9.0"
tag = "2026.9.0"
source-commit = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
published-at = "2026-07-16T10:00:00Z"
release-workflow-identity = "https://github.com/unicity-aos/aos-ce/.github/workflows/release.yml@refs/tags/2026.9.0"

[runtime]
repository = "astrid-runtime/astrid"
version = "2026.9.0"
tag = "v2026.9.0"
release-workflow-identity = "https://github.com/astrid-runtime/astrid/.github/workflows/release.yml@refs/tags/v2026.9.0"
release-metadata-available = true
source-commit = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
release-metadata-asset = "astrid-2026.9.0-release.toml"
release-metadata-blake3 = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"

[contracts]
repository = "astrid-runtime/wit"
commit = "278dbca3e32f327d0f2358644fc86559779ba0fd"
sdk-rust-version = "0.7.1"
sdk-rust-commit = "bbbc61c8821d6c536fb25d2068b6b646e759ad35"

[gates]
release-ready = true
upgrade-self-heal-ready = true
EOF
for metadata_target in aarch64-apple-darwin x86_64-apple-darwin aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu; do
  metadata_asset="unicity-aos-2026.9.0-${metadata_target}.tar.gz"
  cat >> "$fuse_release_metadata" <<EOF

[targets.${metadata_target}]
asset = "${metadata_asset}"
sha256 = "${fuse_asset_sha256}"
blake3 = "${fuse_asset_blake3}"
sigstore-bundle = "${metadata_asset}.sigstore.json"
size = ${fuse_asset_size}
EOF
done
cp "$good_bundle" "$fuse_fixture/valid.sigstore.json"
cp "$good_bundle" "$fuse_fixture/$fuse_asset_name.sigstore.json"
cp "$good_bundle" "$fuse_fixture/unicity-aos-2026.9.0-release.toml.sigstore.json"
cp "$fixture/cosign-linux-amd64" "$fuse_fixture/cosign-linux-amd64"

fuse_home="$work/fuse-home"
mkdir -p "$fuse_home/.astrid"
printf 'standalone-runtime-state\n' > "$fuse_home/.astrid/sentinel"
PATH="$fake_bin:$PATH" HOME="$fuse_home" AOS_TEST_FIXTURE="$fuse_fixture" \
  AOS_VERSION=2026.9.0 sh "$repo_root/install.sh" --yes --no-migrate-prompt >/dev/null
fuse_release_dir="$fuse_home/.aos/releases/2026.9.0"
for binary in \
  astrid astrid-daemon astrid-build astrid-emit \
  astrid-storage-provider-fuse
do
  test -x "$fuse_release_dir/runtime/bin/$binary"
  test ! -e "$fuse_home/.aos/runtime/bin/$binary"
done
test "$(find "$fuse_release_dir/runtime/bin" -mindepth 1 -maxdepth 1 -type f | wc -l | tr -d ' ')" -eq 5
test "$(stat -c '%a' "$fuse_release_dir/runtime/bin/astrid-storage-provider-fuse" 2>/dev/null || stat -f '%Lp' "$fuse_release_dir/runtime/bin/astrid-storage-provider-fuse")" = 700

# Remove only the provider from an otherwise valid signed fixture.  The
# metadata digest and size are updated for this fixture, so the installer gets
# past signature/digest checks and fails closed on its versioned member list
# before touching an existing release or channel pointer.
fuse_missing_fixture="$work/fuse-missing-fixture"
cp -R "$fuse_fixture" "$fuse_missing_fixture"
fuse_missing_tree="$work/fuse-missing-tree"
mkdir "$fuse_missing_tree"
tar -xzf "$fuse_asset" -C "$fuse_missing_tree"
rm "$fuse_missing_tree/unicity-aos-2026.9.0-x86_64-unknown-linux-gnu/runtime/bin/astrid-storage-provider-fuse"
fuse_missing_archive="$fuse_missing_fixture/$fuse_asset_name"
COPYFILE_DISABLE=1 tar -czf "$fuse_missing_archive" \
  -C "$fuse_missing_tree" "unicity-aos-2026.9.0-x86_64-unknown-linux-gnu"
fuse_missing_sha256=$(shasum -a 256 "$fuse_missing_archive" | awk '{print $1}')
fuse_missing_blake3=$(b3sum "$fuse_missing_archive" | awk '{print $1}')
fuse_missing_size=$(wc -c < "$fuse_missing_archive" | tr -d ' ')
python3 - "$fuse_missing_fixture/unicity-aos-2026.9.0-release.toml" \
  "$fuse_missing_sha256" "$fuse_missing_blake3" "$fuse_missing_size" <<'PY'
import pathlib
import sys

path, sha256, blake3, size = sys.argv[1:]
lines = pathlib.Path(path).read_text(encoding="utf-8").splitlines()
inside = False
for index, line in enumerate(lines):
    if line.startswith("["):
        inside = line == "[targets.x86_64-unknown-linux-gnu]"
    if inside:
        if line.startswith("sha256 = "):
            lines[index] = f'sha256 = "{sha256}"'
        elif line.startswith("blake3 = "):
            lines[index] = f'blake3 = "{blake3}"'
        elif line.startswith("size = "):
            lines[index] = f"size = {size}"
pathlib.Path(path).write_text("\n".join(lines) + "\n", encoding="utf-8")
PY
fuse_missing_home="$work/fuse-missing-home"
mkdir -p "$fuse_missing_home/.aos/releases/2026.9.0" \
  "$fuse_missing_home/.aos/update/channels/stable"
printf 'preexisting release\n' > "$fuse_missing_home/.aos/releases/2026.9.0/release-manifest.json"
printf '41\n' > "$fuse_missing_home/.aos/update/channels/stable/current"
if PATH="$fake_bin:$PATH" HOME="$fuse_missing_home" \
  AOS_TEST_FIXTURE="$fuse_missing_fixture" AOS_VERSION=2026.9.0 \
  sh "$repo_root/install.sh" --yes --no-migrate-prompt >/dev/null 2>&1; then
  echo "installer accepted a 2026.9.0 GNU archive missing the FUSE provider" >&2
  exit 1
fi
test "$(cat "$fuse_missing_home/.aos/releases/2026.9.0/release-manifest.json")" = 'preexisting release'
test "$(cat "$fuse_missing_home/.aos/update/channels/stable/current")" = 41
test ! -e "$fuse_missing_home/.aos/runtime"
test ! -e "$fuse_missing_home/.aos/update/install.lock"

unsigned_asset="$work/unsigned-asset.tar.gz"
unsigned_metadata="$work/release-unsigned.toml"
cp "$asset" "$unsigned_asset"
cp "$release_metadata" "$unsigned_metadata"
bundle_root_name="unicity-aos-2026.9.0-x86_64-unknown-linux-gnu"

set_fixture_asset() {
  archive=$1
  previous_sha256=$asset_sha256
  previous_blake3=$asset_blake3
  previous_size=$asset_size
  cp "$archive" "$asset"
  asset_sha256=$(shasum -a 256 "$asset" | awk '{print $1}')
  asset_blake3=$(b3sum "$asset" | awk '{print $1}')
  asset_size=$(wc -c < "$asset" | tr -d ' ')
  sed -i.bak "s/sha256 = \"$previous_sha256\"/sha256 = \"$asset_sha256\"/g" "$release_metadata"
  rm "$release_metadata.bak"
  sed -i.bak "s/blake3 = \"$previous_blake3\"/blake3 = \"$asset_blake3\"/g" "$release_metadata"
  rm "$release_metadata.bak"
  sed -i.bak "s/^size = $previous_size$/size = $asset_size/g" "$release_metadata"
  rm "$release_metadata.bak"
}

restore_unsigned_fixture_asset() {
  cp "$unsigned_asset" "$asset"
  cp "$unsigned_metadata" "$release_metadata"
  asset_sha256=$(shasum -a 256 "$asset" | awk '{print $1}')
  asset_blake3=$(b3sum "$asset" | awk '{print $1}')
  asset_size=$(wc -c < "$asset" | tr -d ' ')
}

signed_tree="$work/signed-tree"
mkdir "$signed_tree"
tar -xzf "$unsigned_asset" -C "$signed_tree"
signed_root="$signed_tree/$bundle_root_name"
printf 'schema-version = 1\nsigned fixture lock\n' > "$signed_root/Distro.lock"
printf 'signed fixture signature\n' > "$signed_root/Distro.sig"
lock_blake3=$(b3sum "$signed_root/Distro.lock" | awk '{print $1}')
sig_blake3=$(b3sum "$signed_root/Distro.sig" | awk '{print $1}')
python3 - "$signed_root/release-manifest.json" "$lock_blake3" "$sig_blake3" <<'PY'
import json
import pathlib
import sys

path, lock_digest, sig_digest = sys.argv[1:]
manifest = json.loads(pathlib.Path(path).read_text(encoding="utf-8"))
manifest["release_files"]["Distro.lock"] = {"blake3": lock_digest, "mode": 0o600}
manifest["release_files"]["Distro.sig"] = {"blake3": sig_digest, "mode": 0o600}
pathlib.Path(path).write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
PY
signed_archive="$work/signed-distro.tar.gz"
COPYFILE_DISABLE=1 tar -czf "$signed_archive" -C "$signed_tree" "$bundle_root_name"
set_fixture_asset "$signed_archive"
signed_home="$work/signed-home"
mkdir -p "$signed_home/.astrid"
printf 'standalone-runtime-state\n' > "$signed_home/.astrid/sentinel"
PATH="$fake_bin:$PATH" HOME="$signed_home" AOS_TEST_FIXTURE="$fixture" \
  AOS_VERSION=2026.9.0 sh "$repo_root/install.sh" --yes --no-migrate-prompt >/dev/null
signed_release_dir="$signed_home/.aos/releases/2026.9.0"
for distro_member in Distro.toml Distro.lock Distro.sig; do
  test -f "$signed_release_dir/$distro_member"
  test "$(stat -c '%a' "$signed_release_dir/$distro_member" 2>/dev/null || stat -f '%Lp' "$signed_release_dir/$distro_member")" = 600
  test ! -e "$signed_home/.aos/runtime/$distro_member"
  test ! -e "$signed_home/.astrid/$distro_member"
done
test "$(cat "$signed_home/.astrid/sentinel")" = standalone-runtime-state

# Signed archives must carry a complete, authenticated Distro inventory.  Keep
# each mutation in the archive manifest so the installer exercises its own
# fail-closed parser rather than a packaging helper.
incomplete_lock_tree="$work/signed-incomplete-lock-tree"
mkdir "$incomplete_lock_tree"
tar -xzf "$signed_archive" -C "$incomplete_lock_tree"
python3 - "$incomplete_lock_tree/$bundle_root_name/release-manifest.json" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
manifest = json.loads(path.read_text(encoding="utf-8"))
manifest["release_files"].pop("Distro.sig")
path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
PY
incomplete_lock_archive="$work/signed-incomplete-lock-distro.tar.gz"
COPYFILE_DISABLE=1 tar -czf "$incomplete_lock_archive" -C "$incomplete_lock_tree" "$bundle_root_name"
set_fixture_asset "$incomplete_lock_archive"
incomplete_lock_home="$work/signed-incomplete-lock-home"
if PATH="$fake_bin:$PATH" HOME="$incomplete_lock_home" AOS_TEST_FIXTURE="$fixture" \
  AOS_VERSION=2026.9.0 sh "$repo_root/install.sh" --yes --no-migrate-prompt >/dev/null 2>&1; then
  echo "installer accepted a signed inventory listing Distro.lock without Distro.sig" >&2
  exit 1
fi
test ! -e "$incomplete_lock_home/.aos/releases/2026.9.0"

incomplete_toml_tree="$work/signed-incomplete-toml-tree"
mkdir "$incomplete_toml_tree"
tar -xzf "$signed_archive" -C "$incomplete_toml_tree"
python3 - "$incomplete_toml_tree/$bundle_root_name/release-manifest.json" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
manifest = json.loads(path.read_text(encoding="utf-8"))
manifest["release_files"].pop("Distro.toml")
path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
PY
incomplete_toml_archive="$work/signed-incomplete-toml-distro.tar.gz"
COPYFILE_DISABLE=1 tar -czf "$incomplete_toml_archive" -C "$incomplete_toml_tree" "$bundle_root_name"
set_fixture_asset "$incomplete_toml_archive"
incomplete_toml_home="$work/signed-incomplete-toml-home"
if PATH="$fake_bin:$PATH" HOME="$incomplete_toml_home" AOS_TEST_FIXTURE="$fixture" \
  AOS_VERSION=2026.9.0 sh "$repo_root/install.sh" --yes --no-migrate-prompt >/dev/null 2>&1; then
  echo "installer accepted a signed inventory listing Distro.lock and Distro.sig without Distro.toml" >&2
  exit 1
fi
test ! -e "$incomplete_toml_home/.aos/releases/2026.9.0"

digest_mismatch_tree="$work/signed-digest-mismatch-tree"
mkdir "$digest_mismatch_tree"
tar -xzf "$signed_archive" -C "$digest_mismatch_tree"
printf 'tampered signed Distro.lock\n' >> "$digest_mismatch_tree/$bundle_root_name/Distro.lock"
digest_mismatch_archive="$work/signed-digest-mismatch-distro.tar.gz"
COPYFILE_DISABLE=1 tar -czf "$digest_mismatch_archive" -C "$digest_mismatch_tree" "$bundle_root_name"
set_fixture_asset "$digest_mismatch_archive"
digest_mismatch_home="$work/signed-digest-mismatch-home"
if PATH="$fake_bin:$PATH" HOME="$digest_mismatch_home" AOS_TEST_FIXTURE="$fixture" \
  AOS_VERSION=2026.9.0 sh "$repo_root/install.sh" --yes --no-migrate-prompt >/dev/null 2>&1; then
  echo "installer accepted a signed Distro member whose bytes disagreed with inventory" >&2
  exit 1
fi
test ! -e "$digest_mismatch_home/.aos/releases/2026.9.0"

mode_mutation_tree="$work/signed-mode-mutation-tree"
mkdir "$mode_mutation_tree"
tar -xzf "$signed_archive" -C "$mode_mutation_tree"
python3 - "$mode_mutation_tree/$bundle_root_name/release-manifest.json" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
manifest = json.loads(path.read_text(encoding="utf-8"))
manifest["release_files"]["Distro.lock"]["mode"] = 0o644
path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
PY
mode_mutation_archive="$work/signed-mode-mutation-distro.tar.gz"
COPYFILE_DISABLE=1 tar -czf "$mode_mutation_archive" -C "$mode_mutation_tree" "$bundle_root_name"
set_fixture_asset "$mode_mutation_archive"
mode_mutation_home="$work/signed-mode-mutation-home"
if PATH="$fake_bin:$PATH" HOME="$mode_mutation_home" AOS_TEST_FIXTURE="$fixture" \
  AOS_VERSION=2026.9.0 sh "$repo_root/install.sh" --yes --no-migrate-prompt >/dev/null 2>&1; then
  echo "installer accepted a signed Distro member with a non-0600 inventory mode" >&2
  exit 1
fi
test ! -e "$mode_mutation_home/.aos/releases/2026.9.0"

malformed_digest_tree="$work/signed-malformed-digest-tree"
mkdir "$malformed_digest_tree"
tar -xzf "$signed_archive" -C "$malformed_digest_tree"
python3 - "$malformed_digest_tree/$bundle_root_name/release-manifest.json" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
manifest = json.loads(path.read_text(encoding="utf-8"))
manifest["release_files"]["Distro.sig"]["blake3"] = "malformed"
path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
PY
malformed_digest_archive="$work/signed-malformed-digest-distro.tar.gz"
COPYFILE_DISABLE=1 tar -czf "$malformed_digest_archive" -C "$malformed_digest_tree" "$bundle_root_name"
set_fixture_asset "$malformed_digest_archive"
malformed_digest_home="$work/signed-malformed-digest-home"
if PATH="$fake_bin:$PATH" HOME="$malformed_digest_home" AOS_TEST_FIXTURE="$fixture" \
  AOS_VERSION=2026.9.0 sh "$repo_root/install.sh" --yes --no-migrate-prompt >/dev/null 2>&1; then
  echo "installer accepted a signed Distro member with a malformed inventory digest" >&2
  exit 1
fi
test ! -e "$malformed_digest_home/.aos/releases/2026.9.0"

missing_tree="$work/signed-missing-tree"
mkdir "$missing_tree"
tar -xzf "$signed_archive" -C "$missing_tree"
rm "$missing_tree/$bundle_root_name/Distro.sig"
missing_archive="$work/signed-missing-distro.tar.gz"
COPYFILE_DISABLE=1 tar -czf "$missing_archive" -C "$missing_tree" "$bundle_root_name"
set_fixture_asset "$missing_archive"
missing_home="$work/signed-missing-home"
if PATH="$fake_bin:$PATH" HOME="$missing_home" AOS_TEST_FIXTURE="$fixture" \
  AOS_VERSION=2026.9.0 sh "$repo_root/install.sh" --yes --no-migrate-prompt >/dev/null 2>&1; then
  echo "installer accepted a signed archive missing Distro.sig" >&2
  exit 1
fi
test ! -e "$missing_home/.aos/releases/2026.9.0"

symlink_tree="$work/signed-symlink-tree"
mkdir "$symlink_tree"
tar -xzf "$signed_archive" -C "$symlink_tree"
rm "$symlink_tree/$bundle_root_name/Distro.sig"
ln -s Distro.lock "$symlink_tree/$bundle_root_name/Distro.sig"
symlink_archive="$work/signed-symlink-distro.tar.gz"
COPYFILE_DISABLE=1 tar -czf "$symlink_archive" -C "$symlink_tree" "$bundle_root_name"
set_fixture_asset "$symlink_archive"
symlink_home="$work/signed-symlink-home"
if PATH="$fake_bin:$PATH" HOME="$symlink_home" AOS_TEST_FIXTURE="$fixture" \
  AOS_VERSION=2026.9.0 sh "$repo_root/install.sh" --yes --no-migrate-prompt >/dev/null 2>&1; then
  echo "installer accepted a signed archive with a Distro.sig symlink" >&2
  exit 1
fi
test ! -e "$symlink_home/.aos/releases/2026.9.0"
restore_unsigned_fixture_asset

python=${PYTHON3:-python3}
"$python" "$repo_root/scripts/release_metadata.py" render-channel \
  --channel stable \
  --generation 2 \
  --published-at 2026-07-16T10:00:00Z \
  --expires-at 2026-08-15T10:00:00Z \
  --release-metadata "$release_metadata" \
  --require-ready \
  --output "$fixture/channel.toml"
cp "$good_bundle" "$fixture/channel.toml.sigstore.json"
cp "$fixture/channel.toml" "$fixture/channel-good.toml"

PATH="$fake_bin:$PATH" HOME="$work/channel-home" AOS_TEST_FIXTURE="$fixture" \
  sh "$repo_root/install.sh" --yes --no-migrate-prompt >/dev/null
accepted_current="$work/channel-home/.aos/update/channels/stable/current"
accepted_channel="$work/channel-home/.aos/update/channels/stable/generations/2/channel.toml"
accepted_bundle="$work/channel-home/.aos/update/channels/stable/generations/2/channel.toml.sigstore.json"
test "$(cat "$accepted_current")" = 2
test -f "$accepted_channel"
test -f "$accepted_bundle"
test "$(awk '$1 == "generation" { print $3 }' "$accepted_channel")" = 2
grep -Fx 'https://github.com/unicity-aos/aos-ce/.github/workflows/promote-channel.yml@refs/heads/main' \
  "$fixture/cosign-identities" >/dev/null
grep -Fx 'https://github.com/unicity-aos/aos-ce/.github/workflows/release.yml@refs/tags/2026.9.0' \
  "$fixture/cosign-identities" >/dev/null

cp "$fixture/channel-good.toml" "$fixture/channel.toml"
nightly_version="2026.9.0-nightly.20260717.gaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
sed -i.bak "s/version = \"2026.9.0\"/version = \"$nightly_version\"/" "$fixture/channel.toml"
rm "$fixture/channel.toml.bak"
if PATH="$fake_bin:$PATH" HOME="$work/nightly-on-stable-home" AOS_TEST_FIXTURE="$fixture" \
  sh "$repo_root/install.sh" --yes --no-migrate-prompt >/dev/null 2>&1; then
  echo "installer accepted a nightly release through the stable channel" >&2
  exit 1
fi
test ! -e "$work/nightly-on-stable-home/.aos"
cp "$fixture/channel-good.toml" "$fixture/channel.toml"

channel_root="$work/channel-home/.aos/update/channels/stable"
mkdir "$channel_root/generations/3"
cp "$accepted_channel" "$channel_root/generations/3/channel.toml"
cp "$accepted_bundle" "$channel_root/generations/3/channel.toml.sigstore.json"
PATH="$fake_bin:$PATH" HOME="$work/channel-home" AOS_TEST_FIXTURE="$fixture" \
  sh "$repo_root/install.sh" --yes --no-migrate-prompt >/dev/null
test "$(cat "$accepted_current")" = 2

mkdir -p "$work/channel-home/.aos/update/install.lock"
sleep 60 &
live_lock_pid=$!
printf '%s\n' "$live_lock_pid" > "$work/channel-home/.aos/update/install.lock/pid"
if PATH="$fake_bin:$PATH" HOME="$work/channel-home" AOS_TEST_FIXTURE="$fixture" \
  sh "$repo_root/install.sh" --yes --no-migrate-prompt >/dev/null 2>&1; then
  echo "installer ignored a live installation lock" >&2
  kill "$live_lock_pid" 2>/dev/null || true
  exit 1
fi
kill "$live_lock_pid" 2>/dev/null || true
wait "$live_lock_pid" 2>/dev/null || true
rm -rf "$work/channel-home/.aos/update/install.lock"
mkdir "$work/channel-home/.aos/update/install.lock"
printf '%s\n' 999999999 > "$work/channel-home/.aos/update/install.lock/pid"
PATH="$fake_bin:$PATH" HOME="$work/channel-home" AOS_TEST_FIXTURE="$fixture" \
  sh "$repo_root/install.sh" --yes --no-migrate-prompt >/dev/null
test ! -e "$work/channel-home/.aos/update/install.lock"

for lexical_case in schema generation size; do
  cp "$fixture/channel-good.toml" "$fixture/channel.toml"
  case "$lexical_case" in
    schema) sed -i.bak 's/schema-version = 1/schema-version = "1"/' "$fixture/channel.toml" ;;
    generation) sed -i.bak 's/generation = 2/generation = "2"/' "$fixture/channel.toml" ;;
    size) sed -E -i.bak 's/^size = ([0-9]+)$/size = "\1"/' "$fixture/channel.toml" ;;
  esac
  rm "$fixture/channel.toml.bak"
  if PATH="$fake_bin:$PATH" HOME="$work/quoted-${lexical_case}-home" AOS_TEST_FIXTURE="$fixture" \
    sh "$repo_root/install.sh" --yes --no-migrate-prompt >/dev/null 2>&1; then
    echo "installer accepted a quoted TOML $lexical_case field" >&2
    exit 1
  fi
  test ! -e "$work/quoted-${lexical_case}-home/.aos"
done

cp "$fixture/release-good.toml" "$release_metadata"
sed -i.bak 's/release-ready = true/release-ready = "true"/' "$release_metadata"
rm "$release_metadata.bak"
if PATH="$fake_bin:$PATH" HOME="$work/quoted-gate-home" AOS_TEST_FIXTURE="$fixture" \
  sh "$repo_root/install.sh" --version 2026.9.0 --yes --no-migrate-prompt >/dev/null 2>&1; then
  echo "installer accepted a quoted TOML readiness gate" >&2
  exit 1
fi
test ! -e "$work/quoted-gate-home/.aos"
cp "$fixture/release-good.toml" "$release_metadata"
cp "$fixture/channel-good.toml" "$fixture/channel.toml"

cp "$fixture/channel-good.toml" "$fixture/channel.toml"
sed -i.bak 's/published-at = "2026-07-16T10:00:00Z"/published-at = "not-a-time"/' "$fixture/channel.toml"
rm "$fixture/channel.toml.bak"
if PATH="$fake_bin:$PATH" HOME="$work/bad-channel-time-home" AOS_TEST_FIXTURE="$fixture" \
  sh "$repo_root/install.sh" --yes --no-migrate-prompt >/dev/null 2>&1; then
  echo "installer accepted one valid and one malformed channel timestamp" >&2
  exit 1
fi
test ! -e "$work/bad-channel-time-home/.aos"

cp "$fixture/channel-good.toml" "$fixture/channel.toml"
sed -i.bak 's/expires-at = "2026-08-15T10:00:00Z"/expires-at = "2026-08-15T10:00:01Z"/' "$fixture/channel.toml"
rm "$fixture/channel.toml.bak"
if PATH="$fake_bin:$PATH" HOME="$work/excessive-channel-lifetime-home" AOS_TEST_FIXTURE="$fixture" \
  sh "$repo_root/install.sh" --yes --no-migrate-prompt >/dev/null 2>&1; then
  echo "installer accepted a channel lifetime beyond its channel maximum" >&2
  exit 1
fi
test ! -e "$work/excessive-channel-lifetime-home/.aos"

cp "$fixture/channel-good.toml" "$fixture/channel.toml"
sed -i.bak 's/published-at = "2026-07-16T10:00:00Z"/published-at = "2026-07-16T10:05:01Z"/' "$fixture/channel.toml"
rm "$fixture/channel.toml.bak"
if PATH="$fake_bin:$PATH" HOME="$work/future-channel-home" AOS_TEST_FIXTURE="$fixture" \
  sh "$repo_root/install.sh" --yes --no-migrate-prompt >/dev/null 2>&1; then
  echo "installer accepted an unreasonably future channel publication" >&2
  exit 1
fi
test ! -e "$work/future-channel-home/.aos"

cp "$fixture/channel-good.toml" "$fixture/channel.toml"
sed -i.bak 's/blake3 = "[0-9a-f]*"/blake3 = "BAD"/' "$fixture/channel.toml"
rm "$fixture/channel.toml.bak"
if PATH="$fake_bin:$PATH" HOME="$work/bad-channel-digest-home" AOS_TEST_FIXTURE="$fixture" \
  sh "$repo_root/install.sh" --yes --no-migrate-prompt >/dev/null 2>&1; then
  echo "installer accepted one valid and one malformed channel target digest" >&2
  exit 1
fi
test ! -e "$work/bad-channel-digest-home/.aos"

cp "$fixture/channel-good.toml" "$fixture/channel.toml"
sed -i.bak 's/generation = 2/generation = 1000000000000000000/' "$fixture/channel.toml"
rm "$fixture/channel.toml.bak"
if PATH="$fake_bin:$PATH" HOME="$work/oversized-generation-home" AOS_TEST_FIXTURE="$fixture" \
  sh "$repo_root/install.sh" --yes --no-migrate-prompt >/dev/null 2>&1; then
  echo "installer accepted a channel generation outside its comparison range" >&2
  exit 1
fi
test ! -e "$work/oversized-generation-home/.aos"

cp "$fixture/channel-good.toml" "$fixture/channel.toml"
sed -i.bak 's/generation = 2/generation = 1/' "$fixture/channel.toml"
rm "$fixture/channel.toml.bak"
if PATH="$fake_bin:$PATH" HOME="$work/channel-home" AOS_TEST_FIXTURE="$fixture" \
  sh "$repo_root/install.sh" --yes --no-migrate-prompt >/dev/null 2>&1; then
  echo "installer accepted a signed channel generation downgrade" >&2
  exit 1
fi
test "$(awk '$1 == "generation" { print $3 }' "$accepted_channel")" = 2

cp "$fixture/channel-good.toml" "$fixture/channel.toml"
sed -i.bak 's/published-at = "2026-07-16T10:00:00Z"/published-at = "2026-07-16T10:00:01Z"/' "$fixture/channel.toml"
rm "$fixture/channel.toml.bak"
if PATH="$fake_bin:$PATH" HOME="$work/channel-home" AOS_TEST_FIXTURE="$fixture" \
  sh "$repo_root/install.sh" --yes --no-migrate-prompt >/dev/null 2>&1; then
  echo "installer accepted conflicting metadata at an accepted channel generation" >&2
  exit 1
fi
cmp "$fixture/channel-good.toml" "$accepted_channel"
cp "$fixture/channel-good.toml" "$fixture/channel.toml"

unavailable_fixture="$work/unavailable-fixture"
mkdir "$unavailable_fixture"
cp "$fixture/cosign-linux-amd64" "$unavailable_fixture/"
if PATH="$fake_bin:$PATH" HOME="$work/unavailable-channel-home" AOS_TEST_FIXTURE="$unavailable_fixture" \
  sh "$repo_root/install.sh" --yes --no-migrate-prompt >/dev/null 2>&1; then
  echo "installer accepted an unavailable default stable channel" >&2
  exit 1
fi
test ! -e "$work/unavailable-channel-home/.aos"

if PATH="$fake_bin:$PATH" HOME="$work/mutually-exclusive-home" AOS_TEST_FIXTURE="$fixture" \
  sh "$repo_root/install.sh" --channel dev --version 2026.9.0 --yes --no-migrate-prompt \
  >/dev/null 2>&1; then
  echo "installer accepted mutually exclusive channel and version selectors" >&2
  exit 1
fi
test ! -e "$work/mutually-exclusive-home/.aos"

printf 'tampered capsule\n' > "$release_dir/capsules/aos-cli.capsule"
PATH="$fake_bin:$PATH" HOME="$work/home" AOS_TEST_FIXTURE="$fixture" AOS_VERSION=2026.9.0 \
  sh "$repo_root/install.sh" --yes --no-migrate-prompt >/dev/null
cmp "$work/capsules/aos-cli.capsule" "$release_dir/capsules/aos-cli.capsule"

cat > "$work/home/.aos/bin/aos" <<'EOF'
#!/bin/sh
set -eu
if [ "${1:-}" = stop ]; then
  : > "$AOS_STOP_MARKER"
fi
echo existing-unicity-aos
EOF
chmod 755 "$work/home/.aos/bin/aos"
cp "$work/home/.aos/bin/aos" "$work/aos-before-unattended-upgrade"
if PATH="$fake_bin:$PATH" HOME="$work/home" AOS_TEST_FIXTURE="$fixture" AOS_VERSION=2026.9.0 \
  AOS_STOP_MARKER="$work/unattended-stop-called" \
  sh "$repo_root/install.sh" --no-migrate-prompt </dev/null >"$work/unattended-upgrade.log" 2>&1; then
  echo "installer replaced an existing installation without confirmation" >&2
  exit 1
fi
cmp "$work/aos-before-unattended-upgrade" "$work/home/.aos/bin/aos"
test ! -e "$work/unattended-stop-called"
grep -F 'rerun with --yes to replace it without a prompt' "$work/unattended-upgrade.log" >/dev/null

rm -f "$fixture/cosign-called"
if PATH="$fake_bin:$PATH" HOME="$work/bad-verifier-home" AOS_TEST_FIXTURE="$fixture" \
  AOS_TEST_BAD_COSIGN_DIGEST=1 AOS_VERSION=2026.9.0 \
  sh "$repo_root/install.sh" --yes --no-migrate-prompt >/dev/null 2>&1; then
  echo "installer accepted a Sigstore verifier with the wrong digest" >&2
  exit 1
fi
test ! -e "$fixture/cosign-called"
test ! -e "$work/bad-verifier-home/.aos"

printf 'invalid Sigstore fixture\n' > "$bundle"
if PATH="$fake_bin:$PATH" HOME="$work/bad-bundle-home" AOS_TEST_FIXTURE="$fixture" AOS_VERSION=2026.9.0 \
  sh "$repo_root/install.sh" --yes --no-migrate-prompt >/dev/null 2>&1; then
  echo "installer accepted an invalid Sigstore bundle" >&2
  exit 1
fi
test ! -e "$work/bad-bundle-home/.aos"
cp "$good_bundle" "$bundle"

mv "$bundle" "$work/missing-bundle"
if PATH="$fake_bin:$PATH" HOME="$work/missing-bundle-home" AOS_TEST_FIXTURE="$fixture" AOS_VERSION=2026.9.0 \
  sh "$repo_root/install.sh" --yes --no-migrate-prompt >/dev/null 2>&1; then
  echo "installer accepted a release with no Sigstore bundle" >&2
  exit 1
fi
test ! -e "$work/missing-bundle-home/.aos"
mv "$work/missing-bundle" "$bundle"

printf 'modified after signing\n' >> "$asset"
if PATH="$fake_bin:$PATH" HOME="$work/modified-asset-home" AOS_TEST_FIXTURE="$fixture" AOS_VERSION=2026.9.0 \
  sh "$repo_root/install.sh" --yes --no-migrate-prompt >/dev/null 2>&1; then
  echo "installer accepted release bytes that did not match the Sigstore bundle" >&2
  exit 1
fi
test ! -e "$work/modified-asset-home/.aos"
cp "$signed_asset" "$asset"

symlink_home="$work/symlink-destination-home"
mkdir -p "$symlink_home/.aos/bin"
cat > "$work/symlink-target" <<'EOF'
#!/bin/sh
set -eu
: > "$AOS_SYMLINK_MARKER"
EOF
chmod 755 "$work/symlink-target"
ln -s "$work/symlink-target" "$symlink_home/.aos/bin/aos"
if PATH="$fake_bin:$PATH" HOME="$symlink_home" AOS_TEST_FIXTURE="$fixture" AOS_VERSION=2026.9.0 \
  AOS_SYMLINK_MARKER="$work/symlink-executed" \
  sh "$repo_root/install.sh" --yes --no-migrate-prompt >/dev/null 2>&1; then
  echo "installer replaced a symlinked destination" >&2
  exit 1
fi
test -L "$symlink_home/.aos/bin/aos"
test ! -e "$work/symlink-executed"

custom_bin_home="$work/custom-bin-home"
custom_bin_target="$work/custom-bin-target"
custom_bin_link="$work/custom-bin-link"
mkdir -p "$custom_bin_home" "$custom_bin_target"
cp "$work/symlink-target" "$custom_bin_target/aos"
ln -s "$custom_bin_target" "$custom_bin_link"
if PATH="$fake_bin:$PATH" HOME="$custom_bin_home" AOS_BIN_DIR="$custom_bin_link" \
  AOS_TEST_FIXTURE="$fixture" AOS_VERSION=2026.9.0 \
  AOS_SYMLINK_MARKER="$work/custom-bin-symlink-executed" \
  sh "$repo_root/install.sh" --yes --no-migrate-prompt >"$work/custom-bin-symlink.log" 2>&1; then
  echo "installer accepted a symlinked custom binary directory" >&2
  exit 1
fi
test ! -e "$work/custom-bin-symlink-executed"
grep -F "refusing symlinked binary directory: $custom_bin_link" "$work/custom-bin-symlink.log" >/dev/null

managed_symlink_home="$work/managed-symlink-home"
mkdir -p "$managed_symlink_home" "$work/managed-symlink-target/bin"
ln -s "$work/managed-symlink-target" "$managed_symlink_home/.aos"
ln -s "$work/symlink-target" "$work/managed-symlink-target/bin/aos"
if PATH="$fake_bin:$PATH" HOME="$managed_symlink_home" AOS_TEST_FIXTURE="$fixture" AOS_VERSION=2026.9.0 \
  AOS_SYMLINK_MARKER="$work/managed-symlink-executed" \
  sh "$repo_root/install.sh" --yes --no-migrate-prompt >/dev/null 2>&1; then
  echo "installer accepted a symlinked managed installation root" >&2
  exit 1
fi
test ! -e "$work/managed-symlink-executed"

run_symlink_home="$work/run-symlink-home"
mkdir -p "$run_symlink_home/.aos" "$work/run-symlink-target"
ln -s "$work/run-symlink-target" "$run_symlink_home/.aos/run"
if PATH="$fake_bin:$PATH" HOME="$run_symlink_home" AOS_TEST_FIXTURE="$fixture" AOS_VERSION=2026.9.0 \
  sh "$repo_root/install.sh" --yes --no-migrate-prompt >/dev/null 2>&1; then
  echo "installer accepted a symlinked AOS run root" >&2
  exit 1
fi

release_bin_symlink_home="$work/release-bin-symlink-home"
mkdir -p "$release_bin_symlink_home/.aos/releases/2026.9.0/runtime" "$work/release-bin-symlink-target"
ln -s "$work/release-bin-symlink-target" \
  "$release_bin_symlink_home/.aos/releases/2026.9.0/runtime/bin"
if PATH="$fake_bin:$PATH" HOME="$release_bin_symlink_home" AOS_TEST_FIXTURE="$fixture" AOS_VERSION=2026.9.0 \
  sh "$repo_root/install.sh" --yes --no-migrate-prompt >/dev/null 2>&1; then
  echo "installer accepted a symlinked release runtime bin directory" >&2
  exit 1
fi

directory_home="$work/directory-destination-home"
mkdir -p "$directory_home/.aos/bin/aos"
if PATH="$fake_bin:$PATH" HOME="$directory_home" AOS_TEST_FIXTURE="$fixture" AOS_VERSION=2026.9.0 \
  sh "$repo_root/install.sh" --yes --no-migrate-prompt >/dev/null 2>&1; then
  echo "installer replaced a directory destination" >&2
  exit 1
fi
test -d "$directory_home/.aos/bin/aos"

for binary in aos astrid astrid-daemon astrid-build astrid-emit; do
  case "$binary" in
    aos) destination="$work/home/.aos/bin/aos" ;;
    *) destination="$release_dir/runtime/bin/$binary" ;;
  esac
  printf '#!/bin/sh\necho old-%s\n' "$binary" > "$destination"
  chmod 755 "$destination"
done
mkdir -p "$work/home/.aos/runtime/bin"
printf '#!/bin/sh\necho stale-mutable-copy\n' > "$work/home/.aos/runtime/bin/astrid"
chmod 755 "$work/home/.aos/runtime/bin/astrid"
printf 'old-release-manifest\n' > "$release_dir/release-manifest.json"

fail_bin="$work/fail-bin"
mkdir "$fail_bin"
cat > "$fail_bin/mv" <<'EOF'
#!/bin/sh
set -eu
last=
for argument in "$@"; do last=$argument; done
if [ "$last" = "$MV_FAIL_DESTINATION" ] && [ ! -f "$MV_FAILED" ]; then
  : > "$MV_FAILED"
  exit 1
fi
exec "$REAL_MV" "$@"
EOF
chmod 755 "$fail_bin/mv"
real_mv=$(command -v mv)
if PATH="$fail_bin:$fake_bin:$PATH" \
  HOME="$work/home" \
  AOS_TEST_FIXTURE="$fixture" \
  AOS_VERSION=2026.9.0 \
  REAL_MV="$real_mv" \
  MV_FAILED="$work/mv-failed" \
  MV_FAIL_DESTINATION="$release_dir" \
  sh "$repo_root/install.sh" --yes --no-migrate-prompt >/dev/null 2>&1; then
  echo "installer ignored a mid-install failure" >&2
  exit 1
fi
for binary in aos astrid astrid-daemon astrid-build astrid-emit; do
  case "$binary" in
    aos) destination="$work/home/.aos/bin/aos" ;;
    *) destination="$release_dir/runtime/bin/$binary" ;;
  esac
  test "$("$destination")" = "old-$binary"
done
test -x "$work/home/.aos/runtime/bin/astrid"
test "$("$work/home/.aos/runtime/bin/astrid")" = stale-mutable-copy
test "$(cat "$release_dir/release-manifest.json")" = old-release-manifest
test "$(find "$release_dir/capsules" -mindepth 1 -maxdepth 1 -type f | wc -l | tr -d ' ')" -eq 22
while IFS= read -r capsule; do
  cmp "$work/capsules/$capsule" "$release_dir/capsules/$capsule"
done < "$release_dir/capsule-assets.txt"
test "$(cat "$work/home/.astrid/sentinel")" = standalone-runtime-state

PATH="$fake_bin:$PATH" HOME="$work/home" AOS_TEST_FIXTURE="$fixture" \
  AOS_VERSION=2026.9.0 sh "$repo_root/install.sh" --yes --no-migrate-prompt >/dev/null
for binary in astrid astrid-daemon astrid-build astrid-emit; do
  test ! -e "$work/home/.aos/runtime/bin/$binary"
  test -x "$release_dir/runtime/bin/$binary"
done

cat > "$work/aos-mismatch" <<'EOF'
#!/bin/sh
if [ "${1:-}" = --version ]; then
  echo 'Unicity AOS 2026.2.0'
fi
EOF
chmod 755 "$work/aos-mismatch"
bash "$repo_root/scripts/package-release.sh" \
  x86_64-unknown-linux-gnu \
  "$work/aos-mismatch" \
  "$work/runtime.tar.gz" \
  0000000000000000000000000000000000000000000000000000000000000000 \
  "$work/capsules" \
  "$fixture" >/dev/null
cp "$asset" "$signed_asset"
if PATH="$fake_bin:$PATH" HOME="$work/mismatch-home" AOS_TEST_FIXTURE="$fixture" AOS_VERSION=2026.9.0 \
  sh "$repo_root/install.sh" --yes --no-migrate-prompt >/dev/null 2>&1; then
  echo "installer accepted a bundle whose binary version did not match the requested release" >&2
  exit 1
fi
test ! -e "$work/mismatch-home/.aos"

unsafe_root="$work/unsafe-bundle/unicity-aos-2026.9.0-x86_64-unknown-linux-gnu"
mkdir -p "$unsafe_root/bin" "$unsafe_root/runtime/bin"
ln -s "$work/aos" "$unsafe_root/bin/aos"
for binary in astrid astrid-daemon astrid-build astrid-emit; do
  cp "$runtime_root/$binary" "$unsafe_root/runtime/bin/$binary"
done
printf '{}\n' > "$unsafe_root/release-manifest.json"
COPYFILE_DISABLE=1 tar -czf "$fixture/unicity-aos-2026.9.0-x86_64-unknown-linux-gnu.tar.gz" \
  -C "$work/unsafe-bundle" "$(basename "$unsafe_root")"
cp "$asset" "$signed_asset"
if PATH="$fake_bin:$PATH" HOME="$work/unsafe-home" AOS_TEST_FIXTURE="$fixture" AOS_VERSION=2026.9.0 \
  sh "$repo_root/install.sh" --yes --no-migrate-prompt >/dev/null 2>&1; then
  echo "installer accepted a symlink in the release archive" >&2
  exit 1
fi
test ! -e "$work/unsafe-home/.aos"
