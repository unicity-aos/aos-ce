#!/usr/bin/env bash
# Sourced by test-install.sh to reuse its package and signed-transport fixtures.
# All fake executables and trust fixtures remain test-only.

for m_target in x86_64-unknown-linux-musl aarch64-unknown-linux-musl; do
  m_root="$work/astrid-$runtime_version-$m_target"
  mkdir -p "$m_root"
  cp "$runtime_root"/* "$m_root/"
  COPYFILE_DISABLE=1 tar -czf "$work/$m_target-runtime.tar.gz" -C "$work" "$(basename "$m_root")"
  bash "$repo_root/scripts/package-release.sh" "$m_target" "$work/aos" \
    "$work/$m_target-runtime.tar.gz" "$(printf '%064d' 0)" "$work/capsules" "$fixture" >/dev/null
  cp "$good_bundle" "$fixture/unicity-aos-2026.9.2-$m_target.tar.gz.sigstore.json"
done
cp "$fixture/cosign-linux-amd64" "$fixture/cosign-linux-arm64"
m_metadata="$fixture/unicity-aos-2026.9.2-musl-release.toml"
PYTHONPATH="$repo_root/scripts" python3 - "$fixture" "$release_metadata" "$repo_root/release/runtime-musl-compatibility.toml" <<'PY'
import hashlib
import pathlib
import subprocess
import sys
import tomllib
import musl_release_metadata as musl

root, legacy_path, pin_path = map(pathlib.Path, sys.argv[1:])
legacy_bytes = legacy_path.read_bytes()
legacy = tomllib.loads(legacy_bytes.decode())
runtime = tomllib.loads(pin_path.read_text())["runtime"]
runtime.pop("release-ready")
runtime["musl-release-metadata-asset"] = f"astrid-{runtime['version']}-musl-release.toml"
runtime["musl-release-metadata-blake3"] = "d" * 64
targets = {}
for target in musl.MUSL_TARGETS:
    path = root / f"unicity-aos-{legacy['version']}-{target}.tar.gz"
    targets[target] = {
        "asset": path.name,
        "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
        "blake3": subprocess.check_output(["b3sum", str(path)], text=True).split()[0],
        "sigstore-bundle": path.name + ".sigstore.json",
        "size": path.stat().st_size,
    }
extension = {
    "schema-version": 1, "kind": musl.KIND,
    "repository": "unicity-aos/aos-ce",
    **{key: legacy[key] for key in ("product", "version", "tag", "source-commit", "release-workflow-identity")},
    "legacy-release": {"metadata-asset": legacy_path.name, "metadata-sha256": hashlib.sha256(legacy_bytes).hexdigest()},
    "runtime-musl": runtime,
    "targets": targets,
}
musl.validate_extension(extension, legacy=legacy, legacy_bytes=legacy_bytes)
(root / f"unicity-aos-{legacy['version']}-musl-release.toml").write_text(musl.render_extension(extension))
PY
cp "$good_bundle" "$m_metadata.sigstore.json"
cp "$m_metadata" "$work/musl-good.toml"

for m_arch in x86_64 aarch64; do
  m_verifier=ae1ecd212663f3693ad9edf8b1a183900c9a52d3155ba6e354237f9a0f6463fc
  if [[ "$m_arch" == aarch64 ]]; then
    m_verifier=2ec865872e331c32fd12b08dae15332d3f92c0aa029219589684a4903ca85d11
  fi
  PATH="$fake_bin:$PATH" HOME="$work/musl-$m_arch-home" AOS_TEST_FIXTURE="$fixture" \
    AOS_TEST_UNAME_M="$m_arch" AOS_TEST_LIBC=musl AOS_TEST_COSIGN_SHA256="$m_verifier" \
    AOS_VERSION=2026.9.2 sh "$repo_root/install.sh" --yes --no-migrate-prompt
  test -x "$work/musl-$m_arch-home/.aos/bin/aos"
done

for m_failure in signature binding duplicate missing; do
  cp "$work/musl-good.toml" "$m_metadata"
  cp "$good_bundle" "$m_metadata.sigstore.json"
  case "$m_failure" in
    signature) printf 'invalid signature\n' > "$m_metadata.sigstore.json" ;;
    binding) sed 's/^metadata-sha256 = .*/metadata-sha256 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"/' "$m_metadata" > "$work/musl-invalid.toml"; cp "$work/musl-invalid.toml" "$m_metadata" ;;
    duplicate) printf '\nsize = 1\n' >> "$m_metadata" ;;
    missing) rm "$m_metadata" ;;
  esac
  if PATH="$fake_bin:$PATH" HOME="$work/musl-$m_failure-home" AOS_TEST_FIXTURE="$fixture" \
    AOS_TEST_LIBC=musl AOS_VERSION=2026.9.2 \
    sh "$repo_root/install.sh" --yes --no-migrate-prompt > "$work/musl-$m_failure.log" 2>&1; then
    echo "musl installer accepted $m_failure extension" >&2; exit 1
  fi
  test ! -e "$work/musl-$m_failure-home/.aos"
done
cp "$work/musl-good.toml" "$m_metadata"
cp "$good_bundle" "$m_metadata.sigstore.json"
