#!/bin/sh
set -eu

if [ "$#" -ne 2 ]; then
    echo "usage: $0 BUILDROOT_OUTPUT OUTPUT_CPIO" >&2
    exit 64
fi

buildroot_output=$(CDPATH='' cd -- "$1" && pwd)
case $2 in
    /*) output_cpio=$2 ;;
    *) output_cpio=$(pwd)/$2 ;;
esac
script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
target_tuple=riscv64-buildroot-linux-gnu
cc=$buildroot_output/host/bin/$target_tuple-gcc
strip=$buildroot_output/host/bin/$target_tuple-strip
cpio=$buildroot_output/host/bin/cpio

if [ ! -x "$cc" ] || [ ! -x "$strip" ] || [ ! -x "$cpio" ]; then
    echo "bootstrap build requires the completed Buildroot host toolchain" >&2
    exit 69
fi
if [ ! -f "$script_dir/buildroot/board/aos/init.c" ]; then
    echo "bootstrap init source is missing" >&2
    exit 66
fi

work_dir=$(mktemp -d "${TMPDIR:-/tmp}/aos-bootstrap.XXXXXX")
trap 'rm -rf "$work_dir"' EXIT HUP INT TERM
mkdir -p \
    "$work_dir/dev" \
    "$work_dir/home/agent" \
    "$work_dir/proc" \
    "$work_dir/run" \
    "$work_dir/sys" \
    "$work_dir/system" \
    "$work_dir/tmp" \
    "$work_dir/workspace"
"$cc" \
    -std=c11 -Os -Wall -Wextra -Werror \
    -march=rv64gc_zicsr_zifencei -mabi=lp64d \
    -static -fno-pie -no-pie \
    -Wl,--build-id=none \
    -o "$work_dir/init" "$script_dir/buildroot/board/aos/init.c"
"$strip" --strip-all "$work_dir/init"
chmod 0755 "$work_dir/init"
chmod 0700 "$work_dir/home/agent" "$work_dir/workspace"
chmod 1777 "$work_dir/tmp"

mkdir -p "$(dirname -- "$output_cpio")"
(
    cd "$work_dir"
    find . -print0 |
        LC_ALL=C sort -z |
        "$cpio" --null --format=newc --owner=0:0 --reproducible --quiet -o
) >"$output_cpio"

sha256sum "$output_cpio"
