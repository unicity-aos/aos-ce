# AOS Linux image

This directory contains the Linux-bearing artifact for `aos-rv64-virt-v0`.
Linux 6.18.39 boots the checked-in Buildroot 2026.05.1 `newc` root filesystem,
runs an AOS-owned static `/init` as PID 1, and keeps one single-hart guest
resident per admitted principal Store.

The current userland is intentionally small but real:

- static musl 1.2.6;
- BusyBox 1.38.0 and its `ash` shell;
- an unprivileged `agent` account at UID/GID 1000 with `/home/agent`;
- a locked root password and no login service;
- `/proc`, `/sys`, static device nodes, and ordinary Linux process/syscall
  execution; and
- no set-user-ID or set-group-ID executables.

The static device contract contains only `/dev/console`, `/dev/null`,
`/dev/zero`, `/dev/random`, and `/dev/urandom`. In particular there is no raw
memory, block, network, graphics, additional TTY, or PTY device node for the
agent shell. The kernel also disables legacy `TIOCSTI`, both PTY families,
networking, raw memory/port devices, input, and media support.

This is not Bash, Debian, or a development distribution yet. It has no Python,
GCC inside the guest, package manager, network device, block device, PTY, or
durable Linux disk. Files under `/home/agent` survive separate calls while the
same Linux RAM remains resident; shutdown, eviction, daemon restart, or a failed
bounded execution discards them. Durable principal storage remains a separate
Realm filesystem until the virtio-block/COW path attaches it.

## Command protocol

PID 1 disables terminal echo and accepts one canonical console line:

```text
AOS/1 <32-lowercase-hex-token> <command>\n
```

It emits token-bound `AOS BEGIN` and `AOS END <token> <status>` frames followed
by `AOS READY`. The capsule generates the 128-bit token from Astrid's host
CSPRNG for every call and accepts only the exact matching terminal frame before
the next ready marker. The shell receives the script, not the token. Its stdin
is `/dev/null`; PID 1 reopens stdout and stderr as write-only console file
descriptions before dropping credentials; and `/dev/console` is root-only. It
runs as UID/GID 1000 with `no_new_privs`, no supplementary groups, no core
dumps, 64 descriptors, at most 32 UID-owned processes, and an 8 MiB file-size
limit.

PID 1 kills and reaps every remaining descendant before it emits the result.
Consequently a background process cannot survive a tool call or write into a
later call's output. The outer capsule independently limits guest RAM, exact
RV64 machine steps, output bytes, and cooperative scheduling slices. The guest
advances only during a metered invocation.

The diagnostic `linux-console` surface admits only `ping`, `counter`, and
`echo`. Arbitrary shell text is sent only by the explicit `linux-sh` action.
`shutdown` powers the guest down through the admitted SBI reset extension.

## Reproducible inputs and builds

`SOURCES.lock` records the verified kernel.org and Buildroot archives, the
Buildroot signing-key fingerprint, exact source/toolchain versions, the pinned
OCI builder, and all retained artifact digests. `build-userland.sh` requires an
exact Buildroot 2026.05.1 tree, verifies the generated RV64IMA/static-musl
configuration before compiling, uses Buildroot's forced package-hash checks,
and rejects a rootfs digest mismatch. `build-image.sh` similarly requires Linux
6.18.39, Clang/LLD 18.1.3, the recorded rootfs, and a clean output directory.

The active Buildroot output directory must be on a Linux filesystem. GNU
package configure probes create path patterns that macOS file-sharing mounts do
not reliably support. A container or Linux CI worker is only the reproducible
build workshop: Docker, QEMU, and host Linux are not present when Astrid runs
the resulting capsule.

Inside the builder recorded in `SOURCES.lock`:

```sh
./build-userland.sh /work/buildroot-2026.05.1 /build/rootfs-out /work/rootfs.cpio
./build-image.sh /work/linux-6.18.39 /work/rootfs.cpio /build/kernel-out /work/Image
```

`build-userland.sh` uses `BR2_DL_DIR` when supplied; otherwise it creates a
sibling cache next to the output directory. It never writes downloads into the
source tree or the container root filesystem.

Both scripts default to fail-closed digest verification. `AOS_RECORD_USERLAND=1`
and `AOS_RECORD_IMAGE=1` only print candidate digests for an intentional source
refresh; recording them still requires review plus an independent clean rebuild.

The checked-in `Image` contains the GPL-2.0-only Linux kernel and GPL-2.0-only
BusyBox; the statically linked musl portions retain musl's MIT license. Release
packaging must retain all notices and make the exact corresponding sources from
`SOURCES.lock` available. `legal-info/` retains the exact target manifest,
generated Buildroot configuration, target license texts, and kernel license;
its README defines the larger corresponding-source release gate.

To exercise the image through the AOS interpreter rather than QEMU:

```sh
cargo test --release -p aos-linux-realm \
  linux_boot_and_console_preserve_userspace_across_invocations -- --nocapture
```

QEMU remains useful only as an independent control experiment. It is not linked
into AOS or required by the product path.
