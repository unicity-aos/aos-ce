#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" -ne 2 ]]; then
  echo "usage: $0 CLEAN_WORK_DIR OUTPUT_DIR" >&2
  exit 64
fi

work=$1
output=$2
realm_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
linux_root="$realm_root/linux"
lock="$linux_root/SOURCES.lock"

lock_value() {
    local value
    value=$(sed -n "s/^$1=//p" "$lock")
    if [[ -z "$value" ]]; then
        echo "SOURCES.lock is missing $1" >&2
        exit 66
    fi
    printf '%s\n' "$value"
}

for tool in curl tar sha256sum b3sum make gcc clang-18; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "origin A requires host tool: $tool" >&2
        exit 69
    fi
done

builder_oci=$(lock_value builder_oci)
expected_builder_oci=ubuntu@sha256:c4a8d5503dfb2a3eb8ab5f807da5bc69a85730fb49b5cfca2330194ebcc41c7b
declared_builder_oci=${AOS_BUILDER_OCI:-}
if [[ "$builder_oci" != "$expected_builder_oci" ]]; then
    echo "SOURCES.lock has an unexpected builder: $builder_oci" >&2
    exit 66
fi
if [[ "$declared_builder_oci" != "$expected_builder_oci" ]]; then
    echo "AOS_BUILDER_OCI must declare $expected_builder_oci" >&2
    exit 66
fi
dpkg_version() {
    dpkg-query -W -f='${Version}' "$1"
}

for package_version in \
    'make=4.3-4.1build2' \
    'gcc=4:13.2.0-7ubuntu1' \
    'cpp-13=13.2.0-23ubuntu4' \
    'gcc-13=13.2.0-23ubuntu4' \
    'g++-13=13.2.0-23ubuntu4' \
    'clang-18=1:18.1.3-1ubuntu1' \
    'llvm-18=1:18.1.3-1ubuntu1' \
    'lld-18=1:18.1.3-1ubuntu1'
do
    package=${package_version%%=*}
    expected=${package_version#*=}
    actual=$(dpkg_version "$package")
    if [[ "$actual" != "$expected" ]]; then
        echo "origin A host package $package is $actual; expected $expected" >&2
        exit 69
    fi
done
if [[ "$(gcc -dumpfullversion)" != 13.2.0 ]] ||
    [[ "$(g++ -dumpfullversion)" != 13.2.0 ]]; then
    echo "origin A selected compiler is not GCC 13.2.0" >&2
    exit 69
fi

[[ -e "$work" ]] && { echo "origin A work directory must not exist: $work" >&2; exit 65; }
mkdir -p "$work/downloads" "$output"

download() {
    local url=$1 destination=$2 expected=$3 actual
    curl --proto '=https' --tlsv1.2 --fail --location --retry 3 \
        -A 'aos-ce-origin-a-build/1.0 (contact: jjb@unicity-labs.com)' \
        --output "$destination" "$url"
    actual=$(sha256sum "$destination" | awk '{print $1}')
    if [[ "$actual" != "$expected" ]]; then
        echo "SHA-256 mismatch for $destination: expected $expected, got $actual" >&2
        exit 70
    fi
}

buildroot_version=$(lock_value buildroot_version)
buildroot_url=$(lock_value buildroot_archive_url)
buildroot_sha=$(lock_value buildroot_archive_sha256)
rust_version=$(lock_value realm_rust)
rust_sha=$(lock_value realm_rust_archive_sha256)
guest_std_sha=$(lock_value guest_tools_riscv_std_sha256)
astrid_url=$(lock_value realm_astrid_build_source_url)
astrid_sha=$(lock_value realm_astrid_build_source_sha256)
astrid_version=$(lock_value realm_astrid_build)
rustup_url=$(lock_value realm_rustup_source_url)
rustup_sha=$(lock_value realm_rustup_source_sha256)
rustup_version=$(lock_value realm_rustup)

download "$buildroot_url" \
    "$work/downloads/buildroot-$buildroot_version.tar.xz" "$buildroot_sha"
download "https://static.rust-lang.org/dist/rust-$rust_version-aarch64-unknown-linux-gnu.tar.xz" \
    "$work/downloads/rust-$rust_version-aarch64-unknown-linux-gnu.tar.xz" "$rust_sha"
download "https://static.rust-lang.org/dist/rust-std-$rust_version-riscv64gc-unknown-linux-gnu.tar.xz" \
    "$work/downloads/rust-std-$rust_version-riscv64gc-unknown-linux-gnu.tar.xz" "$guest_std_sha"
download "$astrid_url" \
    "$work/downloads/astrid-build-$astrid_version.crate" "$astrid_sha"
download "$rustup_url" \
    "$work/downloads/rustup-$rustup_version.tar.gz" "$rustup_sha"

mkdir -p "$work/buildroot-source"
tar -xJf "$work/downloads/buildroot-$buildroot_version.tar.xz" \
    -C "$work/buildroot-source" --strip-components=1

mkdir -p "$work/stage"
"$linux_root/build-userland.sh" \
    "$work/buildroot-source" \
    "$work/buildroot-output" \
    "$work/stage/rootfs-base.cpio.gz"

"$linux_root/prepare-guest-tools.sh" \
    "$work/downloads/rust-$rust_version-aarch64-unknown-linux-gnu.tar.xz" \
    "$work/downloads/rust-std-$rust_version-riscv64gc-unknown-linux-gnu.tar.xz" \
    "$work/downloads/astrid-build-$astrid_version.crate" \
    "$work/downloads/rustup-$rustup_version.tar.gz" \
    "$work/guest-input-work" \
    "$work/guest-inputs"

"$linux_root/build-guest-tools.sh" \
    "$work/guest-inputs/host-rust" \
    "$work/buildroot-output" \
    "$work/guest-inputs/sources/astrid-build-$astrid_version" \
    "$work/guest-inputs/sources/rustup-$rustup_version" \
    "$work/guest-tools-build" \
    "$work/guest-tools"

"$linux_root/assemble-userland.sh" \
    "$work/buildroot-source" \
    "$work/buildroot-output" \
    "$work/guest-tools" \
    "$work/stage/rootfs.cpio.gz" \
    "$output/linux-system.squashfs" \
    "$work/buildroot-output.downloads"

printf 'rootfs_cpio_sha256=%s\n' \
    "$(sha256sum "$work/stage/rootfs.cpio.gz" | awk '{print $1}')"
printf 'linux_system_sha256=%s\n' \
    "$(sha256sum "$output/linux-system.squashfs" | awk '{print $1}')"
printf 'linux_system_blake3=%s\n' \
    "$(b3sum "$output/linux-system.squashfs" | awk '{print $1}')"
