#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 6 ]]; then
  if [[ "${1:-}" != "--sign-release-archive" || $# -ne 3 ]]; then
    echo "usage: $0 <target> <aos-binary> <runtime-archive> <runtime-blake3> <capsule-artifacts> <output-dir>" >&2
    echo "usage: $0 --sign-release-archive <aos-archive> <signed-output-archive>" >&2
    exit 2
  fi
fi
if [[ $# -eq 3 && "${1:-}" != "--sign-release-archive" ]]; then
  exit 2
fi
if ! command -v b3sum >/dev/null 2>&1; then
  echo "required command not found: b3sum" >&2
  exit 1
fi

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

extract_safe_tar() {
  python3 - "$1" "$2" "$3" <<'PY'
import pathlib
import sys
import tarfile

archive, destination, expected_root = sys.argv[1:]
with tarfile.open(archive, "r:gz") as members:
    records = members.getmembers()
    for member in records:
        name = pathlib.PurePosixPath(member.name)
        if name.is_absolute() or ".." in name.parts:
            raise SystemExit(f"unsafe archive path: {member.name}")
        if not (member.isfile() or member.isdir()):
            raise SystemExit(f"unsafe archive member type: {member.name}")
        if expected_root and name.parts and name.parts[0] != expected_root:
            raise SystemExit(f"archive root does not match {expected_root}: {member.name}")
    if expected_root and not any(record.name.startswith(expected_root + "/") for record in records):
        raise SystemExit(f"archive has no {expected_root} payload")
    members.extractall(destination, filter="data")
PY
}

decode_distro_seed() {
  python3 - "$1" <<'PY'
import os
import sys

encoded = os.environ.get("AOS_DISTRO_ED25519_SEED", "")
if len(encoded) != 64 or any(byte not in "0123456789abcdef" for byte in encoded):
    raise SystemExit("AOS_DISTRO_ED25519_SEED must contain 64 lowercase hexadecimal characters")
with open(sys.argv[1], "wb") as seed:
    seed.write(bytes.fromhex(encoded))
PY
  chmod 0600 "$1"
}

destroy_distro_seed() {
  if command -v shred >/dev/null 2>&1; then
    shred -u "$1"
  else
    python3 - "$1" <<'PY'
import os
import sys

path = sys.argv[1]
with open(path, "rb+") as seed:
    size = seed.seek(0, os.SEEK_END)
    seed.seek(0)
    seed.write(b"\0" * size)
    seed.flush()
    os.fsync(seed.fileno())
os.unlink(path)
PY
  fi
}

sign_staged_distro() {
  local staged=$1
  local seed_file=$2
  local shuttle="$SIGNING_WORK/aos-distro.shuttle"
  local mirror="$SIGNING_WORK/aos-distro-signed"
  local seal_log="$SIGNING_WORK/seal.log"
  rm -rf "$mirror"
  mkdir -p "$mirror"
  "$staged/runtime/bin/astrid" distro seal \
    "$staged/Distro.toml" \
    --output "$shuttle" \
    --key "$seed_file" 2> "$seal_log"
  sealer_pubkey_equals_manifest "$staged" "$seal_log"
  extract_safe_tar "$shuttle" "$mirror" ""
  cmp "$staged/Distro.toml" "$mirror/Distro.toml"
  install -m 0600 "$mirror/Distro.lock" "$staged/Distro.lock"
  install -m 0600 "$mirror/Distro.sig" "$staged/Distro.sig"
}

sealer_pubkey_equals_manifest() {
  local staged=$1
  local seal_log=$2
  local declared sealed
  declared=$(python3 - "$staged/Distro.toml" <<'PY'
import pathlib
import sys
try:
    import tomllib
except ModuleNotFoundError:  # Python 3.10 and older
    import tomli as tomllib

pubkey = tomllib.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))["distro"]["signing"][
    "pubkey"
]
if not isinstance(pubkey, str) or not pubkey.startswith("ed25519:"):
    raise SystemExit(f"{sys.argv[1]}: [distro.signing] pubkey must be an ed25519 wire key")
print(pubkey)
PY
  )
  sealed=$(sed -n 's/.*signed by \(ed25519:[A-Za-z0-9+/=_-]*\).*/\1/p' "$seal_log" | tail -n 1)
  if [[ -z "$sealed" ]]; then
    echo "release sealer did not attest its signing public key; refusing to publish" >&2
    exit 1
  fi
  if [[ "$sealed" != "$declared" ]]; then
    echo "sealer public key does not match [distro.signing].pubkey; refusing to publish" >&2
    exit 1
  fi
}

record_signed_distro_inventory() {
  local manifest=$1
  local lock_digest signature_digest
  lock_digest=$(b3sum -- "$(dirname "$manifest")/Distro.lock" | awk '{print $1}')
  signature_digest=$(b3sum -- "$(dirname "$manifest")/Distro.sig" | awk '{print $1}')
  python3 - "$manifest" "$lock_digest" "$signature_digest" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
manifest = json.loads(path.read_text(encoding="utf-8"))
manifest["release_files"]["Distro.lock"] = {"blake3": sys.argv[2], "mode": 384}
manifest["release_files"]["Distro.sig"] = {"blake3": sys.argv[3], "mode": 384}
path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
PY
}

if [[ "${1:-}" == "--sign-release-archive" ]]; then
  archive=$2
  signed_output=$3
  [[ -f "$archive" && ! -L "$archive" ]] || {
    echo "AOS archive is missing or not a regular file: $archive" >&2
    exit 1
  }
  work=$(mktemp -d)
  signing_seed="$work/aos-distro-seed"
  trap '[[ -z "$signing_seed" ]] || destroy_distro_seed "$signing_seed"; rm -rf "$work"' EXIT
  decode_distro_seed "$signing_seed"
  extract_safe_tar "$archive" "$work/extracted" ""
  archive_root=$(find "$work/extracted" -mindepth 1 -maxdepth 1 -type d -print -quit)
  [[ -n "$archive_root" ]] || { echo "AOS archive has no product bundle root" >&2; exit 1; }
  SIGNING_WORK="$work/signing"
  mkdir -p "$SIGNING_WORK"
  sign_staged_distro "$archive_root" "$signing_seed"
  record_signed_distro_inventory "$archive_root/release-manifest.json"
  mkdir -p "$(dirname "$signed_output")"
  COPYFILE_DISABLE=1 tar -czf "$work/signed.tar.gz" -C "$work/extracted" "$(basename "$archive_root")"
  mv "$work/signed.tar.gz" "$signed_output"
  destroy_distro_seed "$signing_seed"
  signing_seed=
  echo "$signed_output"
  exit
fi

target=$1
aos_binary=$2
runtime_archive=$3
runtime_blake3=$4
capsule_artifacts=$5
output_dir=$6

toml_value() {
  python3 - "$1" "$2" "$3" <<'PY'
import pathlib
import sys
try:
    import tomllib
except ModuleNotFoundError:  # Python 3.10 and older
    import tomli as tomllib

path, section, key = sys.argv[1:]
value = tomllib.loads(pathlib.Path(path).read_text(encoding="utf-8"))[section][key]
if not isinstance(value, str) or not value:
    raise SystemExit(f"{path}: [{section}] {key} must be a non-empty string")
print(value)
PY
}

product_version=$(toml_value "$repo_root/crates/unicity-aos-bootstrap/Cargo.toml" package version)
distro_product_version=$(toml_value "$repo_root/distros/community/unicity-ce/Distro.toml" distro version)
runtime_version=$(toml_value "$repo_root/release/runtime-compatibility.toml" runtime version)
runtime_tag=$(toml_value "$repo_root/release/runtime-compatibility.toml" runtime tag)
runtime_repository=$(toml_value "$repo_root/release/runtime-compatibility.toml" runtime repository)
runtime_identity=$(toml_value "$repo_root/release/runtime-compatibility.toml" runtime release-workflow-identity)
wit_repository=$(toml_value "$repo_root/release/runtime-compatibility.toml" contracts repository)
wit_commit=$(toml_value "$repo_root/release/runtime-compatibility.toml" contracts commit)
sdk_rust_version=$(toml_value "$repo_root/release/runtime-compatibility.toml" contracts sdk-rust-version)
sdk_rust_commit=$(toml_value "$repo_root/release/runtime-compatibility.toml" contracts sdk-rust-commit)
asset="unicity-aos-${product_version}-${target}.tar.gz"
root="unicity-aos-${product_version}-${target}"

if [[ -z "$product_version" || -z "$runtime_version" || -z "$runtime_tag" || -z "$runtime_repository" || -z "$runtime_identity" || -z "$wit_repository" || -z "$wit_commit" || -z "$sdk_rust_version" || -z "$sdk_rust_commit" ]]; then
  echo "release compatibility metadata is incomplete" >&2
  exit 1
fi
if [[ "$product_version" != "$distro_product_version" ]]; then
  echo "Cargo product version $product_version does not match Distro version $distro_product_version" >&2
  exit 1
fi
if [[ ! -x "$aos_binary" ]]; then
  echo "AOS binary is missing or not executable: $aos_binary" >&2
  exit 1
fi
if [[ ! -f "$runtime_archive" ]]; then
  echo "runtime archive is missing: $runtime_archive" >&2
  exit 1
fi
if [[ ! "$runtime_blake3" =~ ^[0-9a-f]{64}$ ]]; then
  echo "runtime BLAKE3 digest is malformed" >&2
  exit 1
fi
python3 "$repo_root/scripts/capsule_release.py" --artifacts "$capsule_artifacts"

work=$(mktemp -d)
signing_seed=
trap '[[ -z "$signing_seed" ]] || destroy_distro_seed "$signing_seed"; rm -rf "$work"' EXIT
mkdir -p \
  "$work/runtime-extract" \
  "$work/$root/bin" \
  "$work/$root/libexec" \
  "$work/$root/runtime/bin" \
  "$work/$root/capsules" \
  "$output_dir"

python3 "$repo_root/scripts/validate-runtime-archive.py" \
  "$runtime_archive" \
  "astrid-${runtime_version}-${target}" \
  astrid astrid-daemon astrid-build astrid-emit
tar -xzf "$runtime_archive" -C "$work/runtime-extract"

runtime_root="$work/runtime-extract/astrid-${runtime_version}-${target}"
if [[ ! -d "$runtime_root" ]]; then
  echo "runtime archive has no expected root astrid-${runtime_version}-${target}" >&2
  exit 1
fi

install -m 0755 "$aos_binary" "$work/$root/bin/aos"
install -m 0644 "$repo_root/install.sh" "$work/$root/libexec/install.sh"
for binary in astrid astrid-daemon astrid-build astrid-emit; do
  if [[ ! -x "$runtime_root/$binary" ]]; then
    echo "runtime archive is missing $binary" >&2
    exit 1
  fi
  install -m 0755 "$runtime_root/$binary" "$work/$root/runtime/bin/$binary"
done

python3 "$repo_root/scripts/capsule_release.py" --print-assets > "$work/$root/capsule-assets.txt"
while IFS= read -r capsule; do
  [[ "$capsule" =~ ^aos-[a-z0-9-]+\.capsule$ ]]
  install -m 0644 "$capsule_artifacts/$capsule" "$work/$root/capsules/$capsule"
done < "$work/$root/capsule-assets.txt"
python3 "$repo_root/scripts/capsule_release.py" --artifacts "$work/$root/capsules"

install -m 0644 "$repo_root/release/runtime-compatibility.toml" "$work/$root/runtime-compatibility.toml"
install -m 0644 "$repo_root/distros/community/unicity-ce/Distro.toml" "$work/$root/Distro.toml"
install -m 0644 "$repo_root/README.md" "$work/$root/README.md"

distro_signing=no
if [[ -n "${AOS_DISTRO_ED25519_SEED:-}" ]]; then
  distro_signing=yes
  SIGNING_WORK="$work/signing"
  signing_seed="$work/aos-distro-seed"
  mkdir -p "$SIGNING_WORK"
  decode_distro_seed "$signing_seed"
  sign_staged_distro "$work/$root" "$signing_seed"
  destroy_distro_seed "$signing_seed"
  signing_seed=
fi

release_inventory="$work/release-files.tsv"
: > "$release_inventory"
record_release_file() {
  local relative=$1
  local mode=$2
  local digest
  digest=$(b3sum -- "$work/$root/$relative")
  printf '%s\t%s\t%s\n' "$relative" "$mode" "$(awk '{print $1}' <<<"$digest")" \
    >> "$release_inventory"
}
record_release_file bin/aos 755
record_release_file libexec/install.sh 600
for binary in astrid astrid-daemon astrid-build astrid-emit; do
  record_release_file "runtime/bin/$binary" 755
done
record_release_file capsule-assets.txt 600
record_release_file Distro.toml 600
if [[ "$distro_signing" = yes ]]; then
  record_release_file Distro.lock 600
  record_release_file Distro.sig 600
fi
record_release_file README.md 600
record_release_file runtime-compatibility.toml 600
while IFS= read -r capsule; do
  record_release_file "capsules/$capsule" 600
done < "$work/$root/capsule-assets.txt"

python3 - "$work/$root/release-manifest.json" "$work/$root/capsule-assets.txt" "$release_inventory" "$product_version" "$target" "$runtime_repository" "$runtime_version" "$runtime_tag" "$runtime_blake3" "$runtime_identity" "$wit_repository" "$wit_commit" "$sdk_rust_version" "$sdk_rust_commit" <<'PY'
import json
import pathlib
import sys

path, capsule_list, inventory_path, product, target, runtime_repo, runtime, tag, digest, runtime_identity, wit_repo, wit_commit, sdk_version, sdk_commit = sys.argv[1:]
capsules = pathlib.Path(capsule_list).read_text(encoding="utf-8").splitlines()
release_files = {}
for line in pathlib.Path(inventory_path).read_text(encoding="utf-8").splitlines():
    relative, mode, file_digest = line.split("\t")
    release_files[relative] = {"blake3": file_digest, "mode": int(mode, 8)}
manifest = {
    "schema_version": 2,
    "product": {"name": "Unicity AOS Community Edition", "version": product},
    "target": target,
    "layout": {
        "release_directory": f"releases/{product}",
        "runtime_executables": "runtime/bin",
        "capsule_assets": "capsules",
    },
    "release_files": release_files,
    "runtime": {
        "repository": runtime_repo,
        "version": runtime,
        "tag": tag,
        "asset": f"astrid-{runtime}-{target}.tar.gz",
        "digest": f"blake3:{digest}",
        "release_workflow_identity": runtime_identity,
    },
    "contracts": {
        "repository": wit_repo,
        "commit": wit_commit,
        "sdk_rust_version": sdk_version,
        "sdk_rust_commit": sdk_commit,
    },
    "capsules": {"count": len(capsules), "assets": capsules},
}
verifiers = {
    "aarch64-apple-darwin": {
        "asset": "cosign-darwin-arm64",
        "sha256": "94b42a9e697be95675f6160ab031a9a5f1ec1e646d6f648d7b2f5cd59ececbc5",
    },
    "x86_64-apple-darwin": {
        "asset": "cosign-darwin-amd64",
        "sha256": "14d2678dfbfde18798151e86fbd91ebdadbb7424b18412a42a155dd8a2df4c7a",
    },
    "aarch64-unknown-linux-gnu": {
        "asset": "cosign-linux-arm64",
        "sha256": "2ec865872e331c32fd12b08dae15332d3f92c0aa029219589684a4903ca85d11",
    },
    "x86_64-unknown-linux-gnu": {
        "asset": "cosign-linux-amd64",
        "sha256": "ae1ecd212663f3693ad9edf8b1a183900c9a52d3155ba6e354237f9a0f6463fc",
    },
}
if target not in verifiers:
    raise SystemExit(f"release target has no pinned Sigstore verifier: {target}")
manifest["verifier"] = {"version": "v3.1.1", **verifiers[target]}
pathlib.Path(path).write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
PY

tar -czf "$output_dir/$asset" -C "$work" "$root"
echo "$output_dir/$asset"
