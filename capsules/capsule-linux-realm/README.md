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

`linux_realm_exec` currently admits exactly five nested programs:

- `pwd`
- `echo`
- `write-file`
- `cat`
- `smoke-write`, the original interpreter smoke test

`linux_realm_status` reports the guest-visible mount and command surface without
exposing physical host paths. Every result identifies the kernel-stamped owner
principal and includes the nested process outcome, exit status, stdout, stderr,
fuel consumed, and memory ceiling.

The first execution call lazily creates this layout for the invoking principal;
status reports `uninitialized` before that point without mutating storage:

```text
/home/agent   durable realm home
/workspace    Astrid COW projection of the invocation workspace
/tmp          principal-private temporary subtree
```

`/home/agent` is stored beneath the caller's principal home and survives daemon
restart. `/workspace` is deliberately transactional: writes enter Astrid's
copy-on-write view and do not change the source workspace until the outer Astrid
workflow promotes them. An unpromoted workspace overlay is discarded on daemon
restart. `/tmp` is not durable state.

The default realm name is `default`, giving one durable home per principal. The
principal is never accepted from tool input. It comes from the kernel-stamped
invocation, so two principals using the same capsule bytes receive different
homes.

## Build and install

From the `aos-ce` repository:

```sh
cargo test -p aos-realm-abi -p aos-realm-runtime -p aos-linux-realm \
  --target "$(rustc -vV | sed -n 's/^host: //p')"
cargo clippy -p aos-realm-abi -p aos-realm-runtime -p aos-linux-realm \
  --target "$(rustc -vV | sed -n 's/^host: //p')" -- -D warnings
cargo check -p aos-linux-realm --target wasm32-unknown-unknown
astrid capsule build capsules/capsule-linux-realm
astrid --principal default capsule install \
  capsules/capsule-linux-realm/dist/aos-linux-realm.capsule
```

The installed realm appears as two MCP tools when a current Astrid MCP broker is
present and `astrid --principal default mcp serve` is connected to an MCP client.
The first mutating call may elicit session-ingress consent and a capsule grant.

Example tool arguments:

```json
{"command":"pwd"}
{"command":"echo","args":["hello", "realm"]}
{"command":"write-file","args":["notes.txt","durable\n"],"cwd":"/home/agent"}
{"command":"cat","args":["notes.txt"],"cwd":"/home/agent"}
{"command":"write-file","args":["candidate.rs","..."],"cwd":"/workspace"}
```

Shell syntax is data, not authority. For example, `{"command":"pwd && whoami"}`
is rejected as an unknown program rather than evaluated by Bash.

## Current boundary

The private `aos_realm_v0` ABI supplies bounded argument and CWD reads,
open/read/write/close, monotonic time, and exit. Paths are normalized within
`/home/agent`, `/workspace`, or `/tmp`; unmounted absolute paths and upward escape
fail closed. Writes are buffered at the capsule edge and committed through
Astrid's whole-file VFS operation only when the nested descriptor closes, so a
trapped command does not leave a partial guest file.

This slice does not provide Bash, pipes, child processes, Linux syscalls, a libc,
package management, networking, PTYs, or a compiler. Those belong behind the same
realm boundary; they must not be simulated by granting a host process.

## Distribution direction

The eventual distribution is AOS Realm, not a renamed Debian image. Its signed
base, packages, compiler target, update policy, durable overlay generations, and
build receipts belong to AOS Community Edition. Familiar Linux interfaces are a
compatibility surface. Guest root is never Astrid authority.
