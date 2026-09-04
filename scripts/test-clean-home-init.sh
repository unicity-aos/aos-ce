#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <extracted-product-bundle>" >&2
  exit 2
fi

bundle=$(cd "$1" && pwd -P)
product_version=$(python3 - "$bundle/Distro.toml" <<'PY'
import pathlib
import sys
try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib

with pathlib.Path(sys.argv[1]).open("rb") as file:
    print(tomllib.load(file)["distro"]["version"])
PY
)
for required in bin/aos runtime/bin/astrid runtime/bin/astrid-daemon Distro.toml capsule-assets.txt; do
  [[ -f "$bundle/$required" && ! -L "$bundle/$required" ]] || {
    echo "clean-home init bundle is missing $required" >&2
    exit 1
  }
done
[[ -d "$bundle/capsules" && ! -L "$bundle/capsules" ]] || {
  echo "clean-home init bundle is missing capsules directory" >&2
  exit 1
}

work=$(mktemp -d)
aos_home="$work/user/.aos"
project="$work/project"
mkdir -p "$project" "$aos_home/releases"

release="$aos_home/releases/$product_version"
cp -R "$bundle" "$release"
release=$(cd "$release" && pwd -P)
while IFS= read -r directory; do
  chmod 700 "$directory"
done < <(find "$release" -type d)
while IFS= read -r file; do
  chmod 600 "$file"
done < <(find "$release" -type f)
chmod 700 "$release/bin/aos"
for executable in astrid astrid-daemon astrid-build astrid-emit; do
  chmod 700 "$release/runtime/bin/$executable"
done

# Release CI invokes this gate before the product archive itself has an outer
# Sigstore signature. Construct the same offline signed-inventory shape the
# installer persists so the launcher exercises its complete preflight.
target=x86_64-unknown-linux-gnu
archive_name="unicity-aos-${product_version}-${target}.tar.gz"
archive_root="unicity-aos-${product_version}-${target}"
statement_name="unicity-aos-${product_version}-release.toml"
mkdir -p "$release/signed" "$release/verifier" "$aos_home/bin"
chmod 700 "$release/signed" "$release/verifier" "$aos_home/bin"
verifier="$release/verifier/cosign"
statement="$release/signed/$statement_name"
statement_bundle="$statement.sigstore.json"
archive="$release/signed/$archive_name"
cat > "$verifier" <<EOF
#!/bin/sh
set -eu
bundle=
artifact=
identity=
while [ "\$#" -gt 0 ]; do
  case "\$1" in
    --bundle) bundle=\$2; shift 2 ;;
    --certificate-identity) identity=\$2; shift 2 ;;
    -*) shift ;;
    *) artifact=\$1; shift ;;
  esac
done
[ -n "\$bundle" ] && [ -n "\$artifact" ] && [ -n "\$identity" ]
cmp "\$bundle" "$work/reference-bundle"
cmp "\$artifact" "$work/reference-statement"
[ "\$identity" = "https://github.com/unicity-aos/aos-ce/.github/workflows/release.yml@refs/tags/${product_version}" ]
EOF
chmod 700 "$verifier"
cp "$release/bin/aos" "$aos_home/bin/aos"
chmod 755 "$aos_home/bin/aos"
cmp "$bundle/bin/aos" "$aos_home/bin/aos"

pack="$work/$archive_root"
mkdir -p "$pack"
cp -R "$bundle/." "$pack/"
python3 - "$pack/release-manifest.json" "$verifier" <<'PY'
import hashlib
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
manifest = json.loads(path.read_text(encoding="utf-8"))
manifest["verifier"]["sha256"] = hashlib.sha256(
    pathlib.Path(sys.argv[2]).read_bytes()
).hexdigest()
path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
PY
COPYFILE_DISABLE=1 tar -czf "$archive" -C "$work" "$archive_root"
archive_sha256=$(shasum -a 256 "$archive" | awk '{print $1}')
archive_blake3=$(b3sum "$archive" | awk '{print $1}')
archive_size=$(wc -c < "$archive" | tr -d ' ')
cat > "$statement" <<EOF
schema-version = 1
kind = "aos-release"
product = "unicity-aos-ce"
version = "${product_version}"
tag = "${product_version}"
source-commit = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
published-at = "2026-07-16T10:00:00Z"
release-workflow-identity = "https://github.com/unicity-aos/aos-ce/.github/workflows/release.yml@refs/tags/${product_version}"

[runtime]
release-metadata-available = true
release-metadata-blake3 = "0000000000000000000000000000000000000000000000000000000000000000"

[targets.${target}]
asset = "${archive_name}"
sha256 = "${archive_sha256}"
blake3 = "${archive_blake3}"
sigstore-bundle = "${archive_name}.sigstore.json"
size = ${archive_size}
EOF
printf 'clean-home fixture bundle\n' > "$statement_bundle"
cp "$statement" "$work/reference-statement"
cp "$statement_bundle" "$work/reference-bundle"
chmod 600 "$statement" "$statement_bundle" "$archive"

run_aos() {
  (
    cd "$project"
    HOME="$work/user" \
      AOS_HOME="$aos_home" \
      "$aos_home/bin/aos" "$@"
  )
}

assert_exact_ready_capsules() {
  local ps_json
  ps_json=$(run_aos ps --format json)
  python3 - "$bundle/capsule-assets.txt" "$ps_json" <<'PY'
import json
import pathlib
import sys

assets_path = pathlib.Path(sys.argv[1])
rows = json.loads(sys.argv[2])
expected = sorted(
    line[:-len(".capsule")] if line.endswith(".capsule") else line
    for line in assets_path.read_text(encoding="utf-8").splitlines()
    if line
)
actual = sorted(row.get("capsule") for row in rows)
if actual != expected:
    raise SystemExit(
        f"running capsule set does not match the exact CE release set: "
        f"expected={expected!r}, actual={actual!r}"
    )
not_ready = [row for row in rows if row.get("state") != "ready"]
if not_ready:
    raise SystemExit(f"CE capsules are not all ready: {not_ready!r}")
PY
}

cleanup() {
  status=$?
  trap - EXIT
  run_aos stop >/dev/null 2>&1 || true
  if ! rm -rf "$work"; then
    echo "warning: clean-home fixture cleanup left runtime mounts for runner teardown" >&2
  fi
  exit "$status"
}
trap cleanup EXIT

# Main-only clean-home regression: staged AOS must pass its signed inventory
# preflight and materialize only product-owned state before Astrid owns
# `$AOS_HOME/runtime`. AOS must not stage a compatibility `runtime/bin`.
[[ ! -e "$aos_home/runtime" ]]
[[ ! -e "$aos_home/runtime/bin" ]]
if [[ ! -f "$release/Distro.lock" || ! -f "$release/Distro.sig" ]]; then
  set +e
  run_aos --principal release-gate distro apply --offline --yes \
    --var openai_api_key=release-gate-not-a-real-key \
    > "$work/unsigned-apply.stdout" 2> "$work/unsigned-apply.stderr"
  unsigned_status=$?
  set -e
  if [[ "$unsigned_status" -eq 0 ]]; then
    echo "unpackaged AOS archive must not allow unsigned distribution apply" >&2
    exit 1
  fi
  # Exit 1 is the authenticated-sibling preflight; exit 2 would mean the
  # explicit-principal check rejected the invocation first.
  [[ "$unsigned_status" -eq 1 ]]
  [[ ! -e "$aos_home/runtime" ]]
  echo 'unsigned packaging failed closed; fixture-key apply journey requires signed siblings'
  exit 0
fi

run_aos --principal release-gate distro apply --offline --yes --var openai_api_key=release-gate-not-a-real-key

pin="$aos_home/runtime/trust/unicity-ce.pub"
receipt="$aos_home/receipts/unicity-ce.active.json"
[[ -f "$receipt" && ! -L "$receipt" ]]
[[ "$(find "$aos_home/runtime" -mindepth 1 -maxdepth 1 -print | wc -l | tr -d ' ')" -eq 1 ]]
[[ "$(find "$aos_home/runtime" -mindepth 1 -maxdepth 1 -print -quit)" = "$aos_home/runtime/astrid.volume" ]]
[[ ! -e "$aos_home/runtime/trust" ]]

run_aos start
[[ -f "$pin" && ! -L "$pin" ]]

lock="$aos_home/runtime/home/release-gate/.config/distro.lock"
profile="$aos_home/runtime/etc/profiles/release-gate.toml"
layout_version="$aos_home/runtime/etc/layout-version"
manifest="$release/Distro.toml"
[[ -f "$lock" && -f "$profile" && -f "$layout_version" && ! -L "$layout_version" ]]
[[ -f "$manifest" && ! -L "$manifest" ]]
[[ ! -e "$aos_home/runtime/bin" ]]

python3 - "$manifest" "$release" <<'PY'
import pathlib
import sys

try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib

manifest_path, release_root = map(pathlib.Path, sys.argv[1:])
with manifest_path.open("rb") as file:
    manifest = tomllib.load(file)
sources = [pathlib.Path(item["source"]) for item in manifest["capsule"]]
if not sources:
    raise SystemExit("enforced CE manifest contains no capsule sources")
for source in sources:
    if source.is_absolute() or source.parent != pathlib.PurePath("capsules"):
        raise SystemExit(f"capsule source is not relative to the release root: {source}")
    if source.is_symlink() or not (release_root / source).is_file():
        raise SystemExit(f"capsule source is not a regular release asset: {source}")
PY

python3 - "$bundle/capsule-assets.txt" "$lock" "$profile" <<'PY'
import pathlib
import sys

try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib

assets_path, lock_path, profile_path = map(pathlib.Path, sys.argv[1:])
expected = sorted(
    line[:-len(".capsule")] if line.endswith(".capsule") else line
    for line in assets_path.read_text(encoding="utf-8").splitlines()
    if line
)
if len(expected) != 22 or len(set(expected)) != 22:
    raise SystemExit("release capsule inventory is not the exact 22-capsule CE set")
with lock_path.open("rb") as file:
    lock = tomllib.load(file)
with profile_path.open("rb") as file:
    profile = tomllib.load(file)
if lock.get("distro", {}).get("id") != "unicity-ce":
    raise SystemExit("clean home did not receive the Unicity CE distro lock")
locked = sorted(item["name"] for item in lock.get("capsule", []))
granted = sorted(profile.get("capsules", []))
if locked != expected:
    raise SystemExit("Distro.lock does not bind the exact release capsule set")
if granted != expected:
    raise SystemExit("release principal was not granted the exact release capsule set")
PY

assert_exact_ready_capsules
run_aos doctor

cp "$lock" "$work/distro.lock.before"
cp "$profile" "$work/default.toml.before"
pid_file="$aos_home/run/system.pid"
cli_meta="$aos_home/runtime/home/release-gate/.local/capsules/aos-cli/meta.json"
[[ -f "$pid_file" && ! -L "$pid_file" ]]
[[ -f "$cli_meta" && ! -L "$cli_meta" ]]
cp "$pid_file" "$work/system.pid.before"
cp "$cli_meta" "$work/cli-meta.json.before"
run_aos --principal release-gate init --offline --yes \
  --var openai_api_key=release-gate-not-a-real-key \
  > "$work/pinned-apply.stdout" 2> "$work/pinned-apply.stderr"
grep -Fq 'signature verified against pinned key' "$work/pinned-apply.stderr"
cmp "$work/distro.lock.before" "$lock"
cmp "$work/default.toml.before" "$profile"
cmp "$work/cli-meta.json.before" "$cli_meta"

[[ -d "$project/.aos" ]]
if find "$work/user" "$project" -name .astrid -print -quit | grep -q .; then
  echo "clean AOS initialization created standalone Astrid state" >&2
  exit 1
fi

for transient in system.sock system.pid system.ready system.token mcp-gateway.sock mcp-gateway.ready; do
  [[ ! -e "$aos_home/run/$transient" && ! -L "$aos_home/run/$transient" ]]
done
[[ -f "$aos_home/runtime/astrid.volume" && ! -L "$aos_home/runtime/astrid.volume" && ! -d "$aos_home/runtime/astrid.volume" ]]
[[ "$(find "$aos_home/runtime" -mindepth 1 -maxdepth 1 -print | wc -l | tr -d ' ')" -eq 1 ]]
[[ "$(find "$aos_home/runtime" -mindepth 1 -maxdepth 1 -print -quit)" = "$aos_home/runtime/astrid.volume" ]]
[[ -f "$receipt" && ! -L "$receipt" ]]

echo "clean AOS home initialized, loaded, rechecked, and stopped successfully"
