# AOS Tray

Native macOS menu-bar shell for AOS. Accessory process, no Dock icon. Closing
the panel hides it. Quit AOS Tray does not stop the AOS runtime, mounts,
agents, or MCP sessions.

This is a disconnected first shell. There is no live runtime bridge, no home
read, no networking, no credential collection, no auto-launch, and no
installer or FSKit integration.

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

Default launch (no `--demo`) shows `DISCONNECTED` and empty inventory.

## Claim limits

- Not connected to AOS, Astrid, MCP, or a live principal home.
- Not a consent-enforcement, grantor, updater, or filesystem UI.
- Bundle identity is `ai.unicity.aos.tray`. It must not be confused with
  `org.astrid.runtime.fs`.
- Host/MCP verbs are displayed only when a request lists them. The shell does
  not invent `always` for every request.
- Request identity is the request ID. Same principal, capsule, and scope can
  be distinct invocations.
- Capsule row identity is principal + capsule + scope.
- No UI, VoiceOver, or end-to-end runtime test is claimed here.
