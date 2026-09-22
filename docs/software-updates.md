# Software updates

Open **AOS → Updates**, or press **F2** in `aos console`. Choose Stable, Dev,
or Nightly and select **Check for Updates**. Checking authenticates release
metadata; it does not install packages, start the runtime, or grant permissions.

The first check selects the channel. The Command Center subsequently refreshes
that selection daily while running. It does not choose a new channel for a
legacy installation on first launch. The menu bar shows an update arrow when an
actionable update is known; permission requests take priority.

## What updates together?

- **AOS** includes its bundled Astrid runtime and distribution capsules. Do not
  independently replace that runtime with a standalone Astrid installation.
- **Oracle for Codex, Claude, and Grok** are separate registered plugin snapshots.
  A successful registration does not prove an existing coding session loaded
  the new snapshot. Start a new session when activation is not confirmed.
- **Capsules** are checked for one owned principal at a time. An exact match to
  the verified distribution lock is managed by AOS. Changed or unverifiable
  ownership requires review, not a silent independent update.
- **Standalone Astrid** remains managed by `astrid update --check` and
  `astrid update`. This screen does not search PATH and replace unrelated
  runtimes. Homebrew installations remain managed by Homebrew.

**Update All** installs the actionable AOS/Oracle candidates shown in the
inventory. It excludes independent capsules requiring publisher/capability
review, unsupported sources, and externally managed installations. Capsule
discovery is advisory: GitHub release metadata is not an authenticated archive.
Use the displayed principal-scoped update command to review an independent
capsule through Astrid's existing installer. No `--approve-untrusted` flag is
added automatically.

## Terminal and remote machines

```sh
aos updates list --json                  # cached state, no network
aos updates check --channel stable       # refresh signed metadata
aos updates apply all --yes              # explicit installation
aos updates capsules --principal codex-code --json
```

Run these on the machine containing the installation. A local tray does not
administer a remote SSH host. The JSON and terminal views expose failures and
activation uncertainty separately from an up-to-date result.

Candidates are rechecked before applying. A changed version or archive digest
requires another check. Failed AOS installation stops dependent Oracle updates;
independent host failures remain separate. Retry begins with a fresh check.
Existing sessions are not forcibly restarted.

## Older installations

Older Oracle snapshots do not contain a signed updater or registration record.
Rerun the public Oracle installer once to install that support. A new release
cannot retroactively add code to an old plugin. Similarly, an old AOS updater
without candidate-digest binding is shown as requiring the public installer,
not offered an unsafe in-app apply button.

These features require the corresponding source changes to be released.
Source merges alone do not update installed binaries or existing host sessions.
