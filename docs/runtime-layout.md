# AOS runtime layout

This page distinguishes the layout installed by current `main` from the
accepted single-volume layout for a later AOS `2026.9.0` release. That later
layout is not shipped.

## Current published / `main` layout

AOS product state lives under `~/.aos`. The public launcher is
`~/.aos/bin/aos`. Direct installs authenticate signed channel metadata, then
install a versioned tree under `~/.aos/releases/<version>/` **and** copy the
bundled Astrid executables into `~/.aos/runtime/bin/`.

Current launch still treats `ASTRID_HOME=~/.aos/runtime` as the hosted runtime
home. Typical entries after install include:

```text
~/.aos/bin/aos
~/.aos/libexec/install.sh
~/.aos/releases/<version>/
~/.aos/runtime/bin/astrid
~/.aos/runtime/bin/astrid-daemon
~/.aos/runtime/bin/astrid-build
~/.aos/runtime/bin/astrid-emit
~/.aos/runtime/run/          # sockets, PID, readiness, tokens
~/.aos/update/               # signed channel generations
```

Mutable configuration, keys, principals, capsules, and logs currently remain
native files under the runtime home, not a single stopped-state volume file.
`runtime/run` is the live transient tree. Clean stop does not yet reduce
`~/.aos/runtime` to one `astrid.volume`.

The standalone importer in [runtime-migration.md](runtime-migration.md) still
expects compatibility executables under `runtime/bin` and copies persistent
directory state. It is not the 2026.9.0 single-volume cutover.

## Accepted 2026.9.0 layout (not shipped)

After a clean stop, the accepted runtime directory is exactly:

```text
~/.aos/runtime/astrid.volume
```

That file is the only durable authority under `runtime/`. Signed immutable AOS
and Astrid executables and assets live under `~/.aos/releases/2026.9.0/`.
`~/.aos/bin/aos` and private launcher helpers remain outside `runtime/`.

Process-only sockets, PID files, readiness markers, tokens, gateway state, and
scratch data live under mode-0700 `~/.aos/run/` and are removed completely on
clean stop. Mutable config, keys, principals, capsules, WASM/WIT, audit
records, receipts, and application data are authoritative only inside
`astrid.volume`.

`~/.aos/update` and `~/.aos/migrations` may hold signed installer or update
receipts but never runtime authority. Project-local `.aos` remains workspace
metadata, not a second global runtime home.

This accepted layout depends on unlanded Astrid volume-root and stop work. Do
not treat installer copies under `runtime/bin`, a `runtime/run` tree, or a
draft packaging change as proof that the single-volume contract is live.
