# macOS Command Center packaging

Darwin product archives may carry `share/AOS Command Center.app` as an extra
release member, distinct from the signed filesystem bundle at
`runtime/bin/AstridFS.app`. The Command Center bundle identifier remains
`ai.unicity.aos.tray`. It is not installed to `/Applications` and is not
merged into `AOS.app` / `AstridFS.app`.

## Supply

`scripts/package-release.sh` takes exactly six positional arguments. The
prebuilt app is supplied through `AOS_COMMAND_CENTER_APP`.

- Darwin: the variable is required. The composer fail-closes rather than
  silently omitting the app.
- GNU and musl: the variable is ignored. Those archives do not include the app.

`scripts/package_macos_command_center.py` copies the supplied `.app` unchanged
into `share/AOS Command Center.app`, inventories every regular file, and
rejects symlinks. It does not rewrite Info.plist, re-sign, notarize, or call
Apple signature verifiers. Fixture tests prove byte and mode preservation.

`scripts/build-command-center.sh` is the local/signing policy helper. The
packager does not sign.

- `--mode development --target aarch64-apple-darwin|x86_64-apple-darwin --output DIR`
  builds the named architecture, stamps the product version and display name
  `AOS Command Center` onto the assembled app, and ad-hoc signs it. Source
  `apps/aos-tray/Info.plist` stays the developer preview (`AOS` / `0.0.1`).
  Ad-hoc output is for local tests only and is not production-valid.
- `--mode production --app PATH --output DIR` copies an already signed and
  stapled app, then `codesign --verify --strict --deep` and `stapler validate`.
  Ad-hoc signatures and missing Developer ID Application authority fail closed.
  This mode never runs `codesign --sign`, never calls notary, and never reads
  signing credentials. AOS does not assume Astrid Developer ID material.

Preview CI of `apps/aos-tray` is not packaged GO.

Production acceptance is the same for a later QA rehearsal and a later
release identity: supply an already Developer ID Application signed and
stapled app, then run `--mode production`. Ad-hoc output is never a
production-valid input. AOS does not mint, select, or assume a Team ID
(including Astrid's) and does not call notary. The supplied app is the
trust identity. Fixture packager tests prove byte preservation only.

## Install

When the archive member is present, `install.sh` copies it unchanged into
`$AOS_HOME/releases/<version>/share/AOS Command Center.app`. Archives that
predate this member still install. The installer does not `open` the app, does
not write `/Applications`, and does not create a login item.

Older AOS releases missing the tray remain installable.

## Launch seam (not wired)

The packaged app still requires both `--aos-binary ABSOLUTE` and
`--aos-home ABSOLUTE`. It does not read `AOS_HOME`, PATH, or `LSEnvironment`,
and `autoLaunchAtLogin` remains false. After a Darwin install:

- App bytes: `$AOS_HOME/releases/<version>/share/AOS Command Center.app`
- CLI: `$AOS_BIN_DIR/aos` (default `$AOS_HOME/bin/aos`)
- Home: `$AOS_HOME` (default `$HOME/.aos`)

Proposed later binding (not this increment): a dedicated launch resolver
passes those two absolute paths into the packaged app, for example as
wrapper argv or bundle `LSEnvironment`. It must not invent auth pairing
or execute `aos` from the shell `PATH`. Finder/login-item wiring stays
with that later resolver. Linux and Windows have no Command Center GUI
in this increment.
