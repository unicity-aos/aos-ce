#!/bin/sh
set -eu

if [ "$#" -lt 1 ]; then
    echo "usage: $0 TARGET_DIR" >&2
    exit 64
fi

target_dir=$1

if ! grep -q '^root:\*:' "$target_dir/etc/shadow" ||
   ! grep -q '^agent:\*:' "$target_dir/etc/shadow"; then
    echo "AOS rootfs requires locked root and agent password entries" >&2
    exit 70
fi

# Realm commands already inherit PID 1's console output descriptor. They must
# not be able to reopen the console for input and recover the per-call frame
# token. Keep the static device surface exact: this initramfs has no block,
# network, graphics, PTY, or raw-memory device contract.
for device in console null zero random urandom; do
    if [ ! -c "$target_dir/dev/$device" ]; then
        echo "AOS rootfs is missing required character device /dev/$device" >&2
        exit 70
    fi
done
find "$target_dir/dev" -xdev \( -type b -o -type c \) -print |
while IFS= read -r device; do
    case "$device" in
        "$target_dir/dev/console"|"$target_dir/dev/null"|"$target_dir/dev/zero"|\
        "$target_dir/dev/random"|"$target_dir/dev/urandom") ;;
        *)
            echo "unexpected device in AOS rootfs: $device" >&2
            exit 70
            ;;
    esac
done
chmod 0600 "$target_dir/dev/console"
chmod 0666 "$target_dir/dev/null" "$target_dir/dev/zero" \
    "$target_dir/dev/random" "$target_dir/dev/urandom"

# No executable in this rootfs needs set-user-ID or set-group-ID: the shell is
# permanently constrained by no_new_privs and runs as uid 1000.
find "$target_dir" -xdev -type f -perm /6000 -exec chmod ug-s {} +
privileged=$(find "$target_dir" -xdev -type f -perm /6000 -print -quit)
if [ -n "$privileged" ]; then
    echo "privileged executable remains in AOS rootfs: $privileged" >&2
    exit 70
fi
