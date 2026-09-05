# Linux Realm consumer contract

Status: accepted architecture; **not implemented** on current `main`

Last reviewed: 2026-09-01

Linux Realm is an optional AOS capsule consumer. It is not an authority model,
not a second kernel, and not a prerequisite for the Astrid native kernel
track. `IMPLEMENTABLE_NOW=no` until source-to-artifact provenance, typed
owner/actor projection, and a structured job lifecycle exist.

Current `unicity-aos/aos-ce` `main` does not ship a Linux Realm capsule, guest
image, or `docs` implementation of this contract. Historical RISC-V draft
work and unmerged pull requests are not product authority.

## Independent tracks

- The **native kernel track** owns ring-0 isolation, capabilities, IPC, and
  hardware mediation in Astrid.
- The **Linux Realm track** owns a confined Linux-shaped workbench above the
  stable Astrid capsule boundary.

Either track may proceed without waiting for the other. A hosted or native
Astrid success does not prove Linux Realm, and a Linux Realm prototype does
not prove the native kernel.

## Frontend independence

AOS product frontends, including the Adaptive Shell semantic catalog, are
independent of guest ISA. Shell, MCP, and catalog work must not assume a
Realm architecture or a particular Linux machine.

## Production ISA recommendation

When Linux Realm becomes implementable, the production guest recommendation is
the **host ISA**: `x86_64` first, then `arm64`. That matching-host path is the
only production recommendation recorded here.

RV64-in-WASM remains an inventory and falsifier path. It is not a production
Realm, not a substitute for host-ISA Linux, and not evidence that a RISC-V
machine is the product.

Issue `unicity-aos/aos-ce#76` still describes a RISC-V capsule in its opening
body. Later freeze comments supersede that body for implementation: Linux
remains a confined consumer, provenance is unresolved, and no implementation
should resume from preserved untracked artifacts.

## What Realm may be

A principal-scoped Linux compatibility environment that:

- receives an admitted Astrid view rather than choosing its owner from a path;
- projects workspace and a private home without `host_process` authority;
- keeps Linux syscall emulation, shells, compilers, and package policy out of
  the Astrid native kernel;
- exposes structured jobs, streams, and receipts rather than ambient Bash as
  the product contract.

## What Realm must not be

- a production RISC-V or RV64-in-WASM operating system
- a path-derived authority or second identity model
- a Linux-specific public WIT world
- proof of native-kernel readiness
- a shipped AOS 2026.1.3 feature

## Current honest state

Fleet-computer documentation already states that the fleet computer does not
require Linux and is not itself a Realm. That remains the correct product
boundary until a new signed source-to-artifact Linux Realm candidate exists.
