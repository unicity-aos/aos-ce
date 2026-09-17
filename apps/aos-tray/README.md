# AOS Tray

Native macOS menu-bar shell for AOS. Accessory process, no Dock icon. Closing
the panel hides it. Quit AOS Tray does not stop the AOS runtime, mounts,
agents, or MCP sessions.

Default no-args launch binds the current user's `~/.aos/bin/aos` and `~/.aos`,
or a valid absolute `AOS_HOME`. The binary is always that home's `bin/aos`.
The tray does not search `PATH`, use an app-adjacent executable, or enroll a
device. A missing binary is reported instead of inventing inventory. Explicit
`--aos-binary` / `--aos-home` still select a pair together. There is no
automatic daemon start, app installation or filesystem mount.

## Run

```sh
swift build --package-path apps/aos-tray
swift test --package-path apps/aos-tray
swift run --package-path apps/aos-tray aos-tray --snapshot
swift run --package-path apps/aos-tray aos-tray --demo --snapshot
```

`--snapshot` prints presentation JSON and exits without opening a window.
Omit `--snapshot` to run the menu-bar shell. `--demo` is an in-memory fixture
with a persistent DEMO banner. Demo decisions change the fixture only.

`--socket PATH` is a development presenter listener. It is mutually exclusive
with `--demo` and `--snapshot`. PATH must be an explicit absolute file in an
already-existing user-owned 0700 directory. The process binds that socket
(mode 0600, same-user peer check) before opening UI, serves one newline JSON
request/response per connection, and does not invent principal, capsule, or
scope identities that are absent from the payload. Cancel, timeout,
disconnect, and quit never approve. This is a same-user credential boundary,
not human authenticity proof. Inventory stays unavailable.

`--demo` stays an in-memory fixture. `--socket` stays a presenter without
default-home inventory. No-args launch inspects the bound install when the
binary exists.

To assemble a local app bundle without installing or launching it:

```sh
sh apps/aos-tray/scripts/build-preview.sh
```

The bundle is written under the ignored `.build/AOS Preview.app` directory.
It is ad-hoc signed for local execution, not a Developer ID signed/notarized
distribution. The build verifies that local signature before returning the path.

For an isolated action-dialog fixture after building the preview:

```sh
python3 apps/aos-tray/scripts/preview-action.py
```

This starts only the preview and a temporary private socket. Choosing an option
prints its response and closes the preview; it never contacts a runtime or saves
a grant. The fixture expires after five minutes.

## Verification

The command-center Overview supports explicit read-only inspection:

```sh
aos-tray --aos-binary /absolute/path/to/aos --aos-home /absolute/path/to/aos-home
```

Both arguments are required together and cannot be mixed with demo/snapshot.
Open AOS from the menu bar, or select Overview, to refresh. This executes only
`aos status --json`; it does not start a daemon. Errors show unavailable rather
than stopped. Connected clients and loaded capsules come from the typed status
response, not a complete installed-capsule or agent-session inventory.
Status output is bounded to 64 KiB in memory, with a 15-second monotonic deadline.
A timed-out or oversized status child is killed and reaped; this never signals
the runtime daemon. No status output is written to disk.

The Capsules tab invokes `aos status --json --include-capsules` on the same
explicit installation. It lists names, versions, and descriptions visible to
the authenticated principal, with local search. The view starts at `default`;
“Change…” lists the authenticated user's principals through `aos principals
--json` and the runtime's owned-directory API. Disabled principals cannot be
selected. Unsupported discovery never falls back to a global roster or a
free-text identity. Selection passes the chosen ID through the CLI's
`--principal` option. The returned inventory must match the requested principal.
It does not create agents, enumerate a global roster, or change permissions.
Inventory shares the authenticated status connection and has its own five-second
request timeout (the tray allows 20 seconds for the combined operation).
Stopped, denied, unsupported, and unreachable inventory are not an empty library.
Older AOS builds without this flag remain usable for Overview; Capsules shows
unavailable. The list is package metadata, not effective grants or global inventory.

The volume section reads `runtime/astrid.volume` metadata and can reveal that
container file in Finder. File size is not allocation or capacity. “Open mounted
volume…” separately selects an existing macOS mount root, verifies its lease via
`aos status --json --principal=default --mountpoint=<path>`, and rechecks the native
filesystem before opening Finder. It never starts, mounts, or unmounts a runtime.
Other platforms and older AOS versions refuse this operation. Positive real-mount
navigation still needs execution. This is a development increment, not packaged integration.

The Swift suite covers choice-index preservation, prompt cancellation, socket
isolation, and display metadata. Set `AOS_TRAY_TEST_BINARY` to a built AOS proxy
to enable the Rust/Swift bridge tests as well. Those use scripted runtime replies,
not installed capsules.

The opt-in `scripts/test-native-guest.sh` journey uses a built probe capsule,
disposable homes and explicitly selected candidate binaries. It exercises
private input and daemon restart, separately from scripted socket tests. It does
not by itself prove packaged AOS installation or Windows/Linux native UI.

## Native input setup

With an explicit AOS binary and home, choose **Set Up Native Input…** from the
menu bar. Choose an owned principal and confirm device enrollment and responder
routing. The `aos native-setup` command pairs a dedicated tray key and creates
the private `native-input/connection.json` file under that AOS home. Pairing
tokens travel through standard input, not command-line arguments.

Setup requires a running compatible runtime. It refuses existing enrollment
or responder configuration rather than replacing credentials or policy.
After successful setup, restart the runtime to load its responder configuration
and relaunch the tray. The tray adopts the private connection file from the
explicitly selected home. **Connect Native Input…** also accepts an existing
private connection file; **Reconnect Native Input** retries that connection.

Private text and secret fields use the authenticated native runtime connection,
not the model's MCP result. Input, cancellation and disconnect remain separate
outcomes. The runtime determines the available approval choices and their
lifetimes; selecting a library principal does not grant approval authority.

## Claim limits

- Default launch is not connected to a runtime. Explicit `--socket` mode can
  present requests from the AOS native MCP adapter; it does not read a principal home.
- `--socket` is a local same-user development transport, not a grantor and not
  proof that a human sent the request.
- Native socket presence is not inventory availability.
- Not a consent-enforcement, updater, or filesystem UI.
- Bundle identity is `ai.unicity.aos.tray`. It must not be confused with
  `org.astrid.runtime.fs`.
- Host/MCP verbs are displayed only when a request lists them. The shell does
  not invent `always` for every request.
- Request identity is the request ID. Same principal, capsule, and scope can
  be distinct invocations.
- Capsule row identity is principal + capsule + scope.
- Runtime socket prompts are keyed per connection, not coalesced by request ID.
- GUI, packaged installation and runtime checks must be repeated for the release
  candidate. VoiceOver has not been verified.
