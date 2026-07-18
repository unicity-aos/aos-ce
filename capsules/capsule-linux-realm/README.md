# AOS Realm Capsule

This directory contains the executable seed of AOS Realm: a principal-owned
agent workbench whose guest interface is Linux-shaped and whose outer authority
is an ordinary Astrid capsule.

It is useful now, but it is not Linux yet. The capsule runs signed, embedded core
WebAssembly command modules under Wasmi. The commands receive structured `argv`
and an explicit current directory; there is no host shell command line and the
manifest requests no `host_process` capability.

```text
agent -> realm tool -> nested WASM command -> private realm ABI
      -> mount and descriptor policy -> audited Astrid filesystem imports
```

## What works

`linux_realm_exec` currently admits exactly seven signed workloads:

- `pwd`
- `echo`
- `pipe-echo`, a two-process resumable echo-to-stdin-cat pipeline
- `guest-pipe-echo`, whose supervisor guest creates that pipe and both children,
  then waits for and reaps them itself
- `write-file`
- `cat`
- `smoke-write`, the original interpreter smoke test

`linux_realm_status` reports the guest-visible mount and command surface without
exposing physical host paths. It also reports the caller's actor boot sequence,
completed-command count, next process identifier, and live process/pipe resource
accounting. Every execution result identifies the kernel-stamped owner principal
and includes the nested process outcome, exit status, stdout, stderr, fuel
consumed, memory ceiling, and process identifiers.

The first execution call lazily creates this layout and its in-memory Realm
machine for the invoking principal. Status reports `uninitialized` and an idle
actor before that point without allocating a machine or advancing durable boot
state:

```text
/home/agent   durable realm home
/workspace    Astrid COW projection of the invocation workspace
/tmp          principal-private temporary subtree
```

`/home/agent` is a versioned filesystem. One principal-scoped KV value atomically
selects its current generation; immutable manifests and file contents are stored
as BLAKE3-addressed blobs beneath the caller's private realm store. It survives
daemon restart. `/workspace` is deliberately transactional: writes enter
Astrid's copy-on-write view and do not change the source workspace until the
outer Astrid workflow promotes them. An unpromoted workspace overlay is
discarded on daemon restart. `/tmp` is not durable state.

The default realm name is `default`, giving one durable home per principal. The
principal is never accepted from tool input. It comes from the kernel-stamped
invocation, so two principals using the same capsule bytes receive different
KV heads and different blob namespaces.

## Durable home format

The selected metadata is deliberately small:

```text
principal KV: realm/default/fs/head
  -> { format, generation, manifest_digest }

principal file store: .../store/blobs/<blake3>
  -> immutable file bytes or immutable manifest bytes
```

A create-or-truncate close writes and verifies the file blob, writes and verifies
a new manifest whose parent is the prior selected manifest, then swaps the head
with KV compare-and-swap. A crash before the swap can leave unreachable blobs but
cannot select a partial generation. A losing concurrent writer reloads the winner,
merges its own replacement, and retries up to a fixed bound.

Existing format-0 homes are not discarded. Their direct files are imported into
the versioned store lazily on first read. The old direct path remains a rebuildable
compatibility projection; reads prefer the selected versioned generation.

The current seed supports regular-file create/truncate and read, with a 64 KiB
per-file limit and 1 MiB manifest limit. It does not yet implement delete, rename,
directory metadata, permissions, links, garbage collection, named checkpoints, or
a guest `fsync`/flush call. `linux_realm_status` exposes the format, selected
generation, file count, and manifest digest without exposing a physical path.

## Process-kernel model

`crates/realm-core` is now the host-testable semantic kernel for the next runtime
increment. It provides monotonic typed process and pipe identifiers, explicit
created/runnable/running/waiting/zombie transitions, direct-child wait and reap,
typed signal termination, a single-running-process FIFO reference scheduler,
atomic pipe-descriptor inheritance, bounded partial pipe I/O, backpressure, EOF,
broken-pipe behavior, and aggregate quotas.

The capsule now runs as a long-lived service actor. It admits at most 32 active
principal machines, derives identity only from each kernel-verified message, and
keeps a separate semantic kernel and monotonic PID namespace for each principal.
Completed foreground jobs are reaped, so their process and pipe resources return
to zero while their identifiers are never reused during that capsule boot. A
principal-scoped boot sequence is advanced with KV compare-and-swap and makes PID
reuse after restart explicit as `(boot sequence, PID)`.

The private ABI now exposes bounded `pipe`, `spawn-signed`, `wait`, and `signal`
operations. `guest-pipe-echo` is the first workload whose topology is selected by
guest code: it creates a four-byte pipe, starts signed `stdin-cat` and `echo`
children with exact descriptor mappings, closes its copies, waits for both, and
checks their terminal records. Every process has an isolated Wasmi store and
memory.

This is deliberately narrower than `posix_spawn`. A guest selects an immutable
executable-catalog entry, one bounded UTF-8 argument, and at most one pipe
descriptor mapping. It cannot submit module bytes, inherit Astrid authority, or
start a host process. Process handles encode `(realm boot generation, PID)` and
`wait`/`signal` reject a stale generation or a process that is not the caller's
direct child. The foreground command admits at most two descendants; fuel and
captured-output budgets are partitioned before execution, and all process and pipe
records are deterministically cleaned before the tool result returns. Background
jobs still do not survive calls.

The actor also retains a bounded 64-entry replay window for mutating tool call
IDs. A transport retry with the same principal, call ID, and arguments returns the
recorded result without running the process tree or filesystem mutation again;
reusing the ID with different arguments fails closed. This is an in-memory
per-actor-boot guarantee, not yet a durable exactly-once receipt across crashes.

## Build and install

From the `aos-ce` repository:

```sh
cargo test -p aos-realm-abi -p aos-realm-core -p aos-realm-runtime \
  -p aos-realm-vfs -p aos-linux-realm \
  --target "$(rustc -vV | sed -n 's/^host: //p')"
cargo clippy -p aos-realm-abi -p aos-realm-core -p aos-realm-runtime \
  -p aos-realm-vfs -p aos-linux-realm \
  --target "$(rustc -vV | sed -n 's/^host: //p')" -- -D warnings
cargo check -p aos-linux-realm --target wasm32-unknown-unknown
astrid capsule build capsules/capsule-linux-realm
astrid --principal default capsule install \
  dist/aos-linux-realm.capsule
```

The installed realm appears as two MCP tools when a current Astrid MCP broker is
present and `astrid --principal default mcp serve` is connected to an MCP client.
The first mutating call may elicit session-ingress consent and a capsule grant.

Example tool arguments:

```json
{"command":"pwd"}
{"command":"echo","args":["hello", "realm"]}
{"command":"pipe-echo","args":["hello through two processes"]}
{"command":"guest-pipe-echo","args":["the guest built this pipeline"]}
{"command":"write-file","args":["notes.txt","durable\n"],"cwd":"/home/agent"}
{"command":"cat","args":["notes.txt"],"cwd":"/home/agent"}
{"command":"write-file","args":["candidate.rs","..."],"cwd":"/workspace"}
```

Shell syntax is data, not authority. For example, `{"command":"pwd && whoami"}`
is rejected as an unknown program rather than evaluated by Bash.

## Current boundary

The private `aos_realm_v0` ABI supplies bounded argument and CWD reads,
open/read/write/close, pipe creation, signed child creation, direct-child wait,
direct-child signal, monotonic time, and exit. Paths are normalized within
`/home/agent`, `/workspace`, or `/tmp`; unmounted absolute paths and upward escape
fail closed. Writes are buffered at the capsule edge and committed only when the
nested descriptor closes, so a trapped command does not leave a partial guest
file. Durable-home closes select a content-addressed generation with a KV CAS;
workspace and temporary files retain their outer mount semantics.

This slice does not provide arbitrary executable resolution, full `posix_spawn`
file actions, `execve`, Bash, general Linux syscalls, a libc, package management,
networking, PTYs, background jobs, or a compiler. Those belong behind the same
realm boundary; they must not be simulated by granting a host process.

## Distribution direction

The eventual distribution is AOS Realm, not a renamed Debian image. Its signed
base, packages, compiler target, update policy, durable overlay generations, and
build receipts belong to AOS Community Edition. Familiar Linux interfaces are a
compatibility surface. Guest root is never Astrid authority.
