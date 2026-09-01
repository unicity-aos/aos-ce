#!/usr/bin/env bash
set -euo pipefail

install_mode=false
if [[ "${1:-}" == "--install" ]]; then
    install_mode=true
elif [[ -n "${1:-}" ]]; then
    echo "usage: $0 [--install]" >&2
    exit 64
fi

realm_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
lock="$realm_root/linux/SOURCES.lock"

lock_value() {
    local value
    value=$(sed -n "s/^$1=//p" "$lock")
    if [[ -z "$value" ]]; then
        echo "SOURCES.lock is missing $1" >&2
        exit 66
    fi
    printf '%s\n' "$value"
}

for tool in apt-get awk gpgv sha256sum dpkg-deb; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "host-tool resolution requires: $tool" >&2
        exit 69
    fi
done

snapshot_id=$(lock_value host_tools_snapshot_id)
snapshot_uri=$(lock_value host_tools_snapshot_uri)
suites=$(lock_value host_tools_suites)
components=$(lock_value host_tools_components)
expected_fingerprint=$(lock_value host_tools_archive_key_fingerprint)
expected_keyring_sha=$(lock_value host_tools_keyring_sha256)
expected_tls_ca_sha=$(lock_value host_tools_tls_ca_sha256)
expected_builder_oci=$(lock_value builder_oci)
declared_builder_oci=${AOS_BUILDER_OCI:-}
declared_snapshot_id=${AOS_HOST_TOOLS_SNAPSHOT_ID:-}

if [[ "$declared_builder_oci" != "$expected_builder_oci" ]]; then
    echo "AOS_BUILDER_OCI must declare $expected_builder_oci" >&2
    exit 66
fi
if [[ "$declared_snapshot_id" != "$snapshot_id" ]]; then
    echo "AOS_HOST_TOOLS_SNAPSHOT_ID must declare $snapshot_id" >&2
    exit 66
fi

keyring=/usr/share/keyrings/ubuntu-archive-keyring.gpg
if [[ ! -r "$keyring" ]]; then
    echo "missing Ubuntu archive keyring: $keyring" >&2
    exit 69
fi

tls_ca="$realm_root/linux/snapshot-tls-isrg-root-x1.pem"
if [[ ! -r "$tls_ca" ]]; then
    echo "missing pinned snapshot TLS CA: $tls_ca" >&2
    exit 69
fi
actual_tls_ca_sha=$(sha256sum "$tls_ca" | awk '{print $1}')
if [[ "$actual_tls_ca_sha" != "$expected_tls_ca_sha" ]]; then
    echo "snapshot TLS CA SHA-256 mismatch: expected $expected_tls_ca_sha, got $actual_tls_ca_sha" >&2
    exit 70
fi
actual_keyring_sha=$(sha256sum "$keyring" | awk '{print $1}')
if [[ "$actual_keyring_sha" != "$expected_keyring_sha" ]]; then
    echo "Ubuntu archive keyring SHA-256 mismatch: expected $expected_keyring_sha, got $actual_keyring_sha" >&2
    exit 70
fi

resolution_dir=${AOS_HOST_TOOLS_RESOLUTION_DIR:-${RUNNER_TEMP:-/tmp}/origin-a-host-tools-resolution}
rm -rf "$resolution_dir"
mkdir -p "$resolution_dir"
apt_source="$resolution_dir/aos-snapshot.sources"

{
    echo "Types: deb"
    echo "URIs: $snapshot_uri"
    echo "Suites: $suites"
    echo "Components: $components"
    echo "Signed-By: $keyring"
} > "$apt_source"

APT_OPTIONS=(
    -o Dir::Etc::sourcelist="$apt_source"
    -o Dir::Etc::sourceparts=-
    -o Acquire::Languages=none
    -o Acquire::Retries=3
    -o Acquire::https::snapshot.ubuntu.com::CaInfo="$tls_ca"
)

apt-get "${APT_OPTIONS[@]}" update

: > "$resolution_dir/release-identity.txt"
for suite in $suites; do
    in_release=$(find /var/lib/apt/lists -type f -name "*_dists_${suite}_InRelease" -print -quit)
    if [[ -z "$in_release" ]]; then
        echo "apt did not retain signed InRelease for $suite" >&2
        exit 70
    fi
    fingerprint=$(gpgv --keyring "$keyring" "$in_release" 2>&1 |
        awk '/using RSA key/ { fingerprint=$NF } END { print fingerprint }')
    if [[ "$fingerprint" != "$expected_fingerprint" ]]; then
        echo "$suite signing key mismatch: expected $expected_fingerprint, got ${fingerprint:-none}" >&2
        exit 70
    fi
    release_identity=$(awk -F': ' '$1 ~ /^(Origin|Label|Suite|Codename|Date)$/ {
        printf "%s%s", separator, $0; separator=" | "
    }' "$in_release")
    printf '%s\n' "$release_identity" >> "$resolution_dir/release-identity.txt"
done

package_specs=(
    'build-essential=12.10ubuntu1'
    'make=4.3-4.1build2'
    'gcc=4:13.2.0-7ubuntu1'
    'g++=4:13.2.0-7ubuntu1'
    'clang-18=1:18.1.3-1ubuntu1'
    'llvm-18=1:18.1.3-1ubuntu1'
    'lld-18=1:18.1.3-1ubuntu1'
    'cpp-13=13.2.0-23ubuntu4'
    'gcc-13=13.2.0-23ubuntu4'
    'g++-13=13.2.0-23ubuntu4'
    'gcc-13-base=13.2.0-23ubuntu4'
    'gcc-13-aarch64-linux-gnu=13.2.0-23ubuntu4'
    'g++-13-aarch64-linux-gnu=13.2.0-23ubuntu4'
    'cpp-13-aarch64-linux-gnu=13.2.0-23ubuntu4'
    'libgcc-13-dev=13.2.0-23ubuntu4'
    'libstdc++-13-dev=13.2.0-23ubuntu4'
    # Match snapshot-era runtime dependencies rather than the newer builder
    # base image; the container is disposable and installation is explicit.
    'bzip2=1.0.8-5.1'
    'libbz2-1.0=1.0.8-5.1'
    'gcc-14-base=14-20240412-0ubuntu1'
    'libasan8=14-20240412-0ubuntu1'
    'libatomic1=14-20240412-0ubuntu1'
    'libcc1-0=14-20240412-0ubuntu1'
    'libgomp1=14-20240412-0ubuntu1'
    'libhwasan0=14-20240412-0ubuntu1'
    'libitm1=14-20240412-0ubuntu1'
    'liblsan0=14-20240412-0ubuntu1'
    'libobjc4=14-20240412-0ubuntu1'
    'libtsan2=14-20240412-0ubuntu1'
    'libubsan1=14-20240412-0ubuntu1'
    'libc6=2.39-0ubuntu8.2'
    'libc6-dev=2.39-0ubuntu8.2'
    'libc-dev-bin=2.39-0ubuntu8.2'
    'perl=5.38.2-3.2build2'
    'perl-base=5.38.2-3.2build2'
    'perl-modules-5.38=5.38.2-3.2build2'
)

find /var/cache/apt/archives -maxdepth 1 -type f -name '*.deb' -delete
DEBIAN_FRONTEND=noninteractive apt-get "${APT_OPTIONS[@]}" \
    --allow-downgrades --no-install-recommends --download-only install "${package_specs[@]}"

package_manifest="$resolution_dir/packages.tsv"
hash_manifest="$resolution_dir/packages.sha256"
: > "$package_manifest"
: > "$hash_manifest"
for deb in /var/cache/apt/archives/*.deb; do
    [[ -f "$deb" ]] || continue
    package=$(dpkg-deb -f "$deb" Package)
    version=$(dpkg-deb -f "$deb" Version)
    digest=$(sha256sum "$deb" | awk '{print $1}')
    filename=${deb##*/}
    printf '%s\t%s\t%s\t%s\n' "$package" "$version" "$filename" "$digest" >> "$package_manifest"
    printf '%s  %s\n' "$digest" "$filename" >> "$hash_manifest"
done
sort -o "$package_manifest" "$package_manifest"
sort -o "$hash_manifest" "$hash_manifest"

missing=false
expected_deb_manifest="$realm_root/linux/host-tools-snapshot.debs.sha256"
while read -r expected_digest expected_filename; do
    [[ -n "$expected_digest" ]] || continue
    if ! awk -F'  ' -v digest="$expected_digest" -v filename="$expected_filename" '
        $1 == digest && $2 == filename { found = 1 }
        END { exit !found }
    ' "$hash_manifest"; then
        echo "snapshot .deb mismatch: $expected_filename expected $expected_digest" >&2
        missing=true
    fi
done < "$expected_deb_manifest"
for spec in "${package_specs[@]}"; do
    package=${spec%%=*}
    version=${spec#*=}
    if ! awk -F'\t' -v package="$package" -v version="$version" '
        $1 == package && $2 == version { found = 1 }
        END { exit !found }
    ' "$package_manifest"; then
        echo "snapshot resolution did not download $package=$version" >&2
        missing=true
    fi
done
if [[ "$missing" == true ]]; then
    exit 70
fi

if [[ "$install_mode" == true ]]; then
    DEBIAN_FRONTEND=noninteractive apt-get "${APT_OPTIONS[@]}" \
        --allow-downgrades --no-download --no-install-recommends install "${package_specs[@]}"
fi

{
    echo "snapshot_id=$snapshot_id"
    echo "snapshot_uri=$snapshot_uri"
    echo "suites=$suites"
    echo "components=$components"
    echo "archive_key_fingerprint=$expected_fingerprint"
    echo "snapshot_tls_ca_sha256=$expected_tls_ca_sha"
    echo "builder_oci=$expected_builder_oci"
    echo "install_mode=$install_mode"
} > "$resolution_dir/provenance.txt"

if [[ "$install_mode" == true ]]; then
    dpkg-query -W -f='${Package}\t${Version}\n' \
        build-essential make gcc g++ \
        clang-18 llvm-18 lld-18 \
        cpp-13 gcc-13 g++-13 gcc-13-base \
        gcc-13-aarch64-linux-gnu g++-13-aarch64-linux-gnu \
        cpp-13-aarch64-linux-gnu libgcc-13-dev libstdc++-13-dev \
        > "$resolution_dir/installed.tsv"
    sort -o "$resolution_dir/installed.tsv" "$resolution_dir/installed.tsv"
    gcc -dumpfullversion > "$resolution_dir/gcc-version.txt"
    g++ -dumpfullversion > "$resolution_dir/gxx-version.txt"
fi

echo "resolved locked host tools from Ubuntu snapshot $snapshot_id"
