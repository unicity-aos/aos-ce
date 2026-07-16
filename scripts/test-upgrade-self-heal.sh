#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "required command not found: $1" >&2
    exit 1
  }
}
for command in b3sum cargo find python3 sort stat tar; do
  need "$command"
done

mode_of() {
  stat -c '%a' "$1" 2>/dev/null || stat -f '%Lp' "$1"
}

snapshot_tree() {
  local root=$1
  local output=$2
  : > "$output"
  while IFS= read -r path; do
    printf 'd|%s|%s\n' "$(mode_of "$path")" "${path#"$root"/}" >> "$output"
  done < <(find "$root" -type d -print | LC_ALL=C sort)
  while IFS= read -r path; do
    printf 'f|%s|%s|%s\n' \
      "$(mode_of "$path")" \
      "$(b3sum -- "$path" | awk '{print $1}')" \
      "${path#"$root"/}" >> "$output"
  done < <(find "$root" -type f -print | LC_ALL=C sort)
}

snapshot_shipped_assets() {
  local home=$1
  local output=$2
  local release=$home/releases/2026.1.0
  : > "$output"
  printf '%s|bin/aos\n' "$(b3sum -- "$home/bin/aos" | awk '{print $1}')" >> "$output"
  for name in astrid astrid-daemon astrid-build astrid-emit; do
    printf '%s|runtime/bin/%s\n' \
      "$(b3sum -- "$home/runtime/bin/$name" | awk '{print $1}')" \
      "$name" >> "$output"
  done
  while IFS= read -r capsule; do
    printf '%s|releases/2026.1.0/capsules/%s\n' \
      "$(b3sum -- "$release/capsules/$capsule" | awk '{print $1}')" \
      "$capsule" >> "$output"
  done < "$release/capsule-assets.txt"
}

assert_astrid_untouched() {
  local expected=$1
  local actual=$work/astrid-tree-actual
  snapshot_tree "$legacy" "$actual"
  diff -u "$expected" "$actual"
  test ! -e "$aos_home/imported"
  test ! -e "$aos_home/migrations"
  if grep -R -F "$legacy_marker" "$aos_home" >/dev/null 2>&1; then
    echo "standalone Astrid state was copied into the AOS installation" >&2
    exit 1
  fi
}

home=$work/home
legacy=$home/.astrid
aos_home=$home/.aos
fixture=$work/downloads
fake_bin=$work/fake-bin
capsules=$work/capsules
legacy_marker=standalone-astrid-state-must-remain-external
mkdir -p \
  "$legacy/etc/profiles" \
  "$legacy/home/default/.config/env" \
  "$legacy/home/default/.local/capsules/astrid-mcp" \
  "$legacy/secrets/default/astrid-mcp" \
  "$legacy/var" \
  "$fixture" \
  "$fake_bin" \
  "$capsules"
printf '%s\n' "$legacy_marker" > "$legacy/etc/profiles/default.toml"
printf '{"marker":"%s"}\n' "$legacy_marker" > "$legacy/home/default/.config/env/astrid-mcp.env.json"
printf '%s\n' "$legacy_marker" > "$legacy/home/default/.local/capsules/astrid-mcp/astrid-mcp.capsule"
printf '%s\n' "$legacy_marker" > "$legacy/secrets/default/astrid-mcp/broker_token"
printf '%s\n' "$legacy_marker" > "$legacy/var/state.sentinel"
find "$legacy" -type d -exec chmod 700 {} +
find "$legacy" -type f -exec chmod 600 {} +
chmod 700 "$legacy/home/default/.local/capsules/astrid-mcp/astrid-mcp.capsule"

astrid_before=$work/astrid-tree-before
snapshot_tree "$legacy" "$astrid_before"

cargo build --locked -p unicity-aos-bootstrap --bin aos
product_binary=$repo_root/target/debug/aos
test "$($product_binary --version)" = 'Unicity AOS 2026.1.0'

PYTHONPATH="$repo_root/scripts" python3 - "$capsules" <<'PY'
import pathlib
import sys

from capsule_release import source_contract
from test_capsule_release import write_fixture

output = pathlib.Path(sys.argv[1])
for spec in source_contract():
    write_fixture(output / spec.asset, spec)
PY

target=x86_64-unknown-linux-gnu
runtime_root=$work/astrid-0.9.4-$target
mkdir "$runtime_root"
for name in astrid astrid-daemon astrid-build astrid-emit; do
  printf '#!/bin/sh\necho packaged-%s\n' "$name" > "$runtime_root/$name"
  chmod 755 "$runtime_root/$name"
done
COPYFILE_DISABLE=1 tar -czf "$work/runtime.tar.gz" -C "$work" "$(basename "$runtime_root")"
bash "$repo_root/scripts/package-release.sh" \
  "$target" \
  "$product_binary" \
  "$work/runtime.tar.gz" \
  0000000000000000000000000000000000000000000000000000000000000000 \
  "$capsules" \
  "$fixture" >/dev/null

asset=$fixture/unicity-aos-2026.1.0-$target.tar.gz
bundle=$asset.sigstore.json
printf 'valid Sigstore fixture\n' > "$fixture/valid.sigstore.json"
cp "$fixture/valid.sigstore.json" "$bundle"
asset_sha256=$(shasum -a 256 "$asset" | awk '{print $1}')
asset_blake3=$(b3sum "$asset" | awk '{print $1}')
asset_size=$(wc -c < "$asset" | tr -d ' ')
release_metadata=$fixture/unicity-aos-2026.1.0-release.toml
cat > "$release_metadata" <<EOF
schema-version = 1
kind = "aos-release"
product = "unicity-aos-ce"
version = "2026.1.0"
tag = "2026.1.0"
source-commit = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
published-at = "2026-07-16T10:00:00Z"
release-workflow-identity = "https://github.com/unicity-aos/aos-ce/.github/workflows/release.yml@refs/tags/2026.1.0"

[runtime]
repository = "astrid-runtime/astrid"
version = "0.9.4"
tag = "v0.9.4"
release-workflow-identity = "https://github.com/unicity-astrid/astrid/.github/workflows/release.yml@refs/tags/v0.9.4"
release-metadata-available = false
source-commit = ""
release-metadata-asset = ""
release-metadata-blake3 = ""

[contracts]
repository = "astrid-runtime/wit"
commit = "278dbca3e32f327d0f2358644fc86559779ba0fd"
sdk-rust-version = "0.7.1"
sdk-rust-commit = "bbbc61c8821d6c536fb25d2068b6b646e759ad35"

[gates]
release-ready = false
upgrade-self-heal-ready = false
EOF
for metadata_target in aarch64-apple-darwin x86_64-apple-darwin aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu; do
  metadata_asset="unicity-aos-2026.1.0-${metadata_target}.tar.gz"
  cat >> "$release_metadata" <<EOF

[targets.${metadata_target}]
asset = "${metadata_asset}"
sha256 = "${asset_sha256}"
blake3 = "${asset_blake3}"
sigstore-bundle = "${metadata_asset}.sigstore.json"
size = ${asset_size}
EOF
done
cp "$fixture/valid.sigstore.json" "$release_metadata.sigstore.json"

cat > "$fake_bin/uname" <<'EOF'
#!/bin/sh
case "${1:-}" in
  -s) echo Linux ;;
  -m) echo x86_64 ;;
  *) exit 2 ;;
esac
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
while [ "$#" -gt 0 ]; do
  case "$1" in
    --bundle) bundle=$2; shift ;;
    -*) ;;
    *) artifact=$1 ;;
  esac
  shift
done
cmp "$AOS_TEST_FIXTURE/valid.sigstore.json" "$bundle"
[ -f "$artifact" ]
EOF
cat > "$fake_bin/sha256sum" <<'EOF'
#!/bin/sh
set -eu
case "$1" in
  */cosign)
    printf '%s  %s\n' ae1ecd212663f3693ad9edf8b1a183900c9a52d3155ba6e354237f9a0f6463fc "$1"
    ;;
  *) exec /usr/bin/shasum -a 256 "$1" ;;
esac
EOF
chmod 755 "$fake_bin/uname" "$fake_bin/curl" "$fake_bin/sha256sum" \
  "$fixture/cosign-linux-amd64"

install_candidate() {
  PATH="$fake_bin:$PATH" \
  HOME="$home" \
  AOS_TEST_FIXTURE="$fixture" \
  AOS_VERSION=2026.1.0 \
  sh "$repo_root/install.sh" --yes >/dev/null
}

install_candidate
test -x "$aos_home/bin/aos"
for name in astrid astrid-daemon astrid-build astrid-emit; do
  test -x "$aos_home/runtime/bin/$name"
done
assert_astrid_untouched "$astrid_before"

mkdir -p "$aos_home/runtime/var"
printf 'private AOS runtime state\n' > "$aos_home/runtime/var/state.sentinel"
chmod 600 "$aos_home/runtime/var/state.sentinel"
state_before=$(b3sum "$aos_home/runtime/var/state.sentinel" | awk '{print $1}')

shipped_before=$work/shipped-before
shipped_after=$work/shipped-after
snapshot_shipped_assets "$aos_home" "$shipped_before"
for name in aos astrid astrid-daemon astrid-build astrid-emit; do
  case "$name" in
    aos) destination=$aos_home/bin/aos ;;
    *) destination=$aos_home/runtime/bin/$name ;;
  esac
  printf '#!/bin/sh\nexit 0\n' > "$destination"
  chmod 755 "$destination"
done
while IFS= read -r capsule; do
  printf 'tampered capsule\n' > "$aos_home/releases/2026.1.0/capsules/$capsule"
done < "$aos_home/releases/2026.1.0/capsule-assets.txt"
chmod 755 \
  "$aos_home" \
  "$aos_home/bin" \
  "$aos_home/runtime" \
  "$aos_home/runtime/bin" \
  "$aos_home/releases" \
  "$aos_home/releases/2026.1.0" \
  "$aos_home/releases/2026.1.0/capsules"

install_candidate
snapshot_shipped_assets "$aos_home" "$shipped_after"
diff -u "$shipped_before" "$shipped_after"
test "$(b3sum "$aos_home/runtime/var/state.sentinel" | awk '{print $1}')" = "$state_before"
assert_astrid_untouched "$astrid_before"

for directory in \
  "$aos_home" \
  "$aos_home/bin" \
  "$aos_home/runtime" \
  "$aos_home/runtime/bin" \
  "$aos_home/releases" \
  "$aos_home/releases/2026.1.0" \
  "$aos_home/releases/2026.1.0/capsules"; do
  test "$(mode_of "$directory")" = 700
done

if bash "$repo_root/scripts/test-final-runtime-boot.sh" \
  "$aos_home/bin/aos" \
  "$aos_home" > "$work/pending-boot.log" 2>&1; then
  echo "final runtime boot gate ran before an exact runtime was approved" >&2
  exit 1
fi
grep -F 'the exact approved runtime is not release-ready' "$work/pending-boot.log" >/dev/null

echo "packaged clean install, reinstall, and self-heal checks passed"
