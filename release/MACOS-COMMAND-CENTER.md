# macOS Command Center packaging and signing

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

## Production signing

Darwin release cells assemble, Developer ID sign, notarize, staple, then
production-validate the app before compose:

1. `scripts/build-command-center.sh --mode development --target <darwin-triple> --output DIR`
   swift-builds the named architecture and ad-hoc signs a local app. Ad-hoc
   output is not production-valid.
2. `scripts/sign-command-center.sh --app PATH --output DIR` copies that app,
   imports `AOS_MACOS_CERTIFICATE_P12_PATH` into an ephemeral keychain,
   codesigns it as `ai.unicity.aos.tray` with `--keychain` bound to that
   keychain plus `--options runtime --timestamp`, zips with `ditto`
   **before** `notarytool submit`, then staples and validates. It never
   calls `security find-identity`, never exports key material, and never
   changes the runner default or search-list keychain. The keychain is
   deleted on exit.
3. `scripts/build-command-center.sh --mode production --app PATH --output DIR`
   copies the stapled app and fail-closes unless `codesign --verify` shows
   Developer ID Application authority and `stapler validate` succeeds. This
   mode never signs, notarizes, or reads credentials.
4. The production app path is exported as `AOS_COMMAND_CENTER_APP` into the
   existing six-argument `package-release.sh` compose. Linux GNU and musl
   cells leave the variable unset.

The tag/dispatch `build` job is not a `pull_request` workflow and does not
put the whole matrix behind `environment: release`. Apple credentials are
wired only on Darwin cells, fail closed when empty, and use AOS-owned names.
A dedicated `aos-macos-signing` GitHub Environment is a later ruling.

These names are the contract. This repository does not store their values,
and this increment does not claim the secrets are configured:

| Helper environment | Release mapping |
| --- | --- |
| `AOS_MACOS_DEVELOPMENT_TEAM_ID` | `secrets.AOS_MACOS_DEVELOPMENT_TEAM_ID` |
| `AOS_MACOS_DEVELOPER_ID_IDENTITY` | `secrets.AOS_MACOS_DEVELOPER_ID_IDENTITY` |
| `AOS_MACOS_CERTIFICATE_P12_PATH` | workflow base64-decodes `secrets.AOS_MACOS_CERTIFICATE_P12` to `$RUNNER_TEMP/aos-command-center-signing.p12`, mode `0600`, then unsets the secret environment variable |
| `AOS_MACOS_CERTIFICATE_PASSWORD` | `secrets.AOS_MACOS_CERTIFICATE_PASSWORD` |
| `AOS_MACOS_NOTARY_KEY_ID` | `secrets.AOS_MACOS_NOTARY_KEY_ID` |
| `AOS_MACOS_NOTARY_ISSUER_ID` | `secrets.AOS_MACOS_NOTARY_ISSUER_ID` |
| `AOS_MACOS_NOTARY_KEY_PATH` | workflow writes `secrets.AOS_MACOS_NOTARY_KEY` to `$RUNNER_TEMP`, mode `0600`, then unsets the secret environment variable |
| `AOS_MACOS_NOTARY_PROFILE` | optional local keychain profile; unused on the API-key CI path |
| `AOS_MACOS_TOOL_DIR` | test override; default `/usr/bin` |

There is no silent ad-hoc identity, no `Developer ID Application` default, and
no fallback to `ASTRID_MACOS_*` or `ASTRID_FSKIT_*`. Missing AOS credentials
fail closed. Creating the org/repo Apple secrets is reserved authority; none
are assumed present.

`scripts/test_sign_command_center.sh` covers fail-closed credentials,
missing PKCS12 path/password, Astrid refusal, ad-hoc refusal, tool-dir
wrappers, exact-line team/identifier checks (including longer spoof
values), ephemeral import plus `--keychain` binding and keychain
cleanup, and the `release.yml` contract. It is not a live Apple
notarization. Secrets are not configured in this increment; creating
them remains reserved authority.

Preview CI of `apps/aos-tray` is not packaged GO.

## Install

When the archive member is present, `install.sh` copies it unchanged into
`$AOS_HOME/releases/<version>/share/AOS Command Center.app`. Archives that
predate this member still install. The installer does not `open` the app, does
not write `/Applications`, and does not create a login item.

Older AOS releases missing the tray remain installable.

## Launch

No-args launch binds the current user's `~/.aos/bin/aos` and `~/.aos`. A valid
absolute `AOS_HOME` overrides the home; the binary is always that home's
`bin/aos`. PATH and app-adjacent executables are not searched. Missing binaries
are reported rather than guessed.

`--aos-binary ABSOLUTE` and `--aos-home ABSOLUTE` still select an explicit
pair together. `autoLaunchAtLogin` remains false. After a Darwin install:

- App bytes: `$AOS_HOME/releases/<version>/share/AOS Command Center.app`
- Default CLI: `$HOME/.aos/bin/aos`
- Default home: `$HOME/.aos`

Linux and Windows have no Command Center GUI in this increment.
