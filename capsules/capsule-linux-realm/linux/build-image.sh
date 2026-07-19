#!/bin/sh
set -eu

if [ "$#" -lt 2 ] || [ "$#" -gt 3 ]; then
    echo "usage: $0 LINUX_SOURCE BUILD_DIR [OUTPUT_IMAGE]" >&2
    exit 64
fi

kernel_source=$1
build_dir=$2
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
output_image=${3:-"$script_dir/Image"}
expected_init=7d2341377ba960cb3db791096e6746b025b0c65be02e21fc5e279605ae0dc61c
expected_image=fd81101a96b7bccfdaa1a0f451cf828df3126f337e0dfbd13db37495cf206982
record_image=${AOS_RECORD_IMAGE:-0}

if [ "$record_image" != 0 ] && [ "$record_image" != 1 ]; then
    echo "AOS_RECORD_IMAGE must be 0 or 1" >&2
    exit 64
fi

if [ -e "$build_dir" ]; then
    echo "BUILD_DIR must not already exist: $build_dir" >&2
    exit 65
fi
if [ ! -x "$kernel_source/scripts/config" ]; then
    echo "not an extracted Linux source tree: $kernel_source" >&2
    exit 66
fi
if [ "$(make -s -C "$kernel_source" kernelversion)" != "6.18.39" ]; then
    echo "build-image.sh requires exact Linux 6.18.39 sources" >&2
    exit 66
fi
if ! clang --version | head -n 1 | grep -q '18\.1\.3'; then
    echo "build-image.sh requires Clang 18.1.3 for the recorded image" >&2
    exit 69
fi
if ! ld.lld --version | head -n 1 | grep -q '18\.1\.3'; then
    echo "build-image.sh requires LLD 18.1.3 for the recorded image" >&2
    exit 69
fi

mkdir -p "$build_dir"
init="$build_dir/aos-init"
initramfs="$build_dir/initramfs.list"

clang --target=riscv64-unknown-linux-gnu \
    -march=rv64ima_zicsr_zifencei -mabi=lp64 \
    -nostdlib -static -fuse-ld=lld \
    -Wl,--build-id=none -Wl,-Ttext=0x10000 \
    -o "$init" "$script_dir/init.S"
actual_init=$(sha256sum "$init" | cut -d ' ' -f 1)
if [ "$record_image" = 0 ] && [ "$actual_init" != "$expected_init" ]; then
    echo "init digest mismatch: expected $expected_init, got $actual_init" >&2
    exit 70
fi
sed "s|@AOS_INIT@|$init|" "$script_dir/initramfs.list" > "$initramfs"

make -C "$kernel_source" O="$build_dir/kernel" \
    ARCH=riscv LLVM=1 LLVM_IAS=1 tinyconfig
"$kernel_source/scripts/config" --file "$build_dir/kernel/.config" \
    --enable 64BIT \
    --enable MMU \
    --enable RISCV_SBI \
    --enable NONPORTABLE \
    --disable SMP \
    --disable RISCV_ISA_C \
    --disable FPU \
    --disable RISCV_ISA_V \
    --disable MODULES \
    --disable EFI \
    --disable HIBERNATION \
    --disable CPU_IDLE \
    --enable PRINTK \
    --enable TTY \
    --enable HVC_DRIVER \
    --enable HVC_RISCV_SBI \
    --enable SERIAL_EARLYCON_RISCV_SBI \
    --enable BINFMT_ELF \
    --enable DEVTMPFS \
    --enable DEVTMPFS_MOUNT \
    --enable BLK_DEV_INITRD \
    --set-str INITRAMFS_SOURCE "$initramfs" \
    --enable INITRAMFS_COMPRESSION_NONE \
    --disable INITRAMFS_COMPRESSION_GZIP \
    --disable DEBUG_INFO \
    --disable DEBUG_INFO_BTF \
    --disable WERROR

export KBUILD_BUILD_USER=aos
export KBUILD_BUILD_HOST=builder
export KBUILD_BUILD_VERSION=1
export KBUILD_BUILD_TIMESTAMP='Thu Jan  1 00:00:00 UTC 1970'
export SOURCE_DATE_EPOCH=0

make -C "$kernel_source" O="$build_dir/kernel" \
    ARCH=riscv LLVM=1 LLVM_IAS=1 olddefconfig
make -j8 -C "$kernel_source" O="$build_dir/kernel" \
    ARCH=riscv LLVM=1 LLVM_IAS=1 Image

image="$build_dir/kernel/arch/riscv/boot/Image"
actual_image=$(sha256sum "$image" | cut -d ' ' -f 1)
if [ "$record_image" = 0 ] && [ "$actual_image" != "$expected_image" ]; then
    echo "image digest mismatch: expected $expected_image, got $actual_image" >&2
    exit 70
fi
cp "$image" "$output_image"
if [ "$record_image" = 1 ]; then
    printf 'init_elf_sha256=%s\nimage_sha256=%s\n' "$actual_init" "$actual_image"
fi
printf '%s  %s\n' "$actual_image" "$output_image"
