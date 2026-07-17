# AOS Linux Realm Capsule

This directory is the executable seed of AOS Realm: a principal-owned,
persistent agent workbench whose guest interface is Linux-shaped and whose
outer authority is an ordinary Astrid capsule.

The current slice is intentionally smaller than Linux. It runs a nested core
WebAssembly guest under Wasmi through three private guest imports: `write`,
`clock-monotonic-ns`, and `exit`. Execution has fixed fuel, memory, instance,
table, and output limits. The capsule requests no `host_process` capability.

That establishes the recursive isolation boundary:

```text
agent -> realm tool -> nested guest ABI -> realm policy -> Astrid imports
```

It does not yet establish POSIX conformance, Bash, a package manager, or Linux
binary compatibility.

The realm is intended to become the agent's default shell and build service. It is
not embedded into every capsule: callers submit explicit, bounded jobs, while
ordinary capsules continue to use narrower HTTP, filesystem, identity, and model
interfaces. A delegated job receives only the intersection of caller, capsule,
realm, and per-job authority.

## Distribution direction

The eventual distribution is AOS Realm, not a repackaged Debian image. Its base
images, packages, compiler target, update policy, and artifact receipts belong
to the AOS Community Edition product. Familiar Linux interfaces are
compatibility surfaces; guest root is never Astrid authority.

Planned distribution properties include:

- an immutable, signed base with a principal-private writable overlay;
- a durable `/home/agent` and separately granted `/workspace`;
- transactional package changes with content-addressed inputs and rollback;
- explicit network, secret, workspace, and export portals;
- a machine-readable view of the realm's effective authority;
- build receipts connecting inputs, toolchain, output, and verification.

## Verify the seed

```sh
cargo test -p aos-realm-abi -p aos-realm-runtime -p aos-linux-realm
cargo clippy -p aos-realm-abi -p aos-realm-runtime -p aos-linux-realm -- -D warnings
cargo check -p aos-linux-realm --target wasm32-unknown-unknown
```

Package the component with `astrid capsule build capsules/capsule-linux-realm`.
The seed is deliberately not selected by the signed Unicity CE distribution
until it has principal-scoped persistence and a useful guest toolchain.
