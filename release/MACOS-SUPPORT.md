# macOS support floors

AOS on Darwin has three separate floors. Do not treat Finder/FSKit
availability as a requirement for installing or running AOS. Do not treat
a Darwin compiler setting as proof that a packaged binary, or a live older
Mac, will run.

| Surface | Packaged minimum | Evidence | Native proof |
| --- | --- | --- | --- |
| CLI (`bin/aos`) | Mach-O `LC_BUILD_VERSION` minos 11.0 | AOS 2026.9.3 Darwin archive `unicity-aos-2026.9.3-aarch64-apple-darwin`. The Darwin release job also sets `MACOSX_DEPLOYMENT_TARGET=11.0` for the `aos` crate only. | Untested on macOS 11–15. This checkout host is macOS 26.6.2. Installer fixtures cover skip/copy dispatch, not native execution. |
| Bundled runtime (`astrid`, `astrid-daemon`, `astrid-build`, `astrid-emit`, `astrid-storage-provider-fskit`) | Mach-O `LC_BUILD_VERSION` minos 11.0 | Same AOS 2026.9.3 Darwin archive, and independently Astrid v2026.9.4 `astrid-2026.9.4-aarch64-apple-darwin`. This floor is the packaged Astrid Mach-O, not the AOS deployment-target env. | Untested on macOS 11–15. |
| Command Center tray (`aos-tray`) | bundle `LSMinimumSystemVersion` 13.0 (Mach-O minos 13.0) | Production `apps/aos-tray/Info.plist` and packaged `aos-tray`. | Untested on macOS 13–15. The installer copies the app only when the host is at or above that bundle minimum. |
| Optional Finder volume mount (`AstridFS.app`) | bundle `LSMinimumSystemVersion` 26.0 (Mach-O minos 26.0) | Signed `AstridFS.app`. Astrid's path-backed FSKit resource (`FSPathURLResource`) is macOS 26+. `FSUnaryFileSystem` at 15.4 is not sufficient. | Required by the Apple API. No macOS FUSE fallback. |

macOS 15.4 can host a unary FSKit filesystem, but this product mounts a path
URL, not a block device. Lowering AstridFS to 15.4 would not make mounts work.

The Darwin archive still ships `astrid-storage-provider-fskit` and
`AstridFS.app`. That is packaging, not a mount requirement. Installers must
not fail the whole product when the current OS cannot register or mount that
extension.

When the host is older than the bundled `AstridFS.app`
`LSMinimumSystemVersion` (currently 26.0):

- `install.sh` still installs the runtime and capsules and exits 0.
- It explains that the extension cannot be registered or approved, skips
  `aos-filesystem.sh install/enable`, and does not launch the manager.
- Unknown host versions or unreadable bundle minima keep the previous
  install+enable fail-closed path.
- Supported hosts with a genuine manager failure still exit nonzero with
  `macOS filesystem setup is incomplete`.
- `aos storage mount` remains fail-closed.

Command Center copy is independent of the FSKit skip and follows the tray
bundle minimum (currently 13.0):

- Host older than that minimum: skip copy into `~/Applications` and skip
  `open`. The app remains in the immutable release `share/` directory. CLI
  install still exits 0.
- Host at or above that minimum: keep the current atomic copy/open. Copy/mv
  failures stay fatal; a failed `open` still warns and continues.
- Unknown host versions or unreadable Command Center minima keep the
  previous copy/open path.

No claim that macOS 11–15 executes the CLI, daemon, or tray. Those are
packaged Mach-O floors plus installer dispatch tests.

Re-run the filesystem manager after upgrading:

```sh
/bin/sh "$AOS_HOME/releases/<version>/runtime/bin/macos/aos-filesystem.sh" install
/bin/sh "$AOS_HOME/releases/<version>/runtime/bin/macos/aos-filesystem.sh" enable
```
