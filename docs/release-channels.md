# Signed AOS release channels

AOS resolves direct installs through signed `stable`, `dev`, and `nightly`
channel pointers. The default is `stable`:

```sh
curl --proto '=https' --tlsv1.2 -fsSL https://aos.unicity.ai/install.sh | sh
curl --proto '=https' --tlsv1.2 -fsSL https://aos.unicity.ai/install.sh | sh -s -- --channel dev
```

An exact release is a separate, mutually exclusive operation:

```sh
sh install.sh --version 2026.1.0
```

There is no GitHub `releases/latest` fallback. If a selected channel has not
been published, is expired, has an invalid signature, or conflicts with locally
accepted state, installation stops before writing `AOS_HOME`.

## Trust and metadata

Every immutable release publishes:

- `unicity-aos-<version>-<target>.tar.gz` and its Sigstore bundle;
- `unicity-aos-<version>-release.toml` and its Sigstore bundle; and
- BLAKE3 and SHA-256 checksum manifests.

The strict release document records its tag, source commit, exact release
workflow identity, four target assets and digests, compatibility pins, and the
two release-readiness gates. The exact accepted identity is:

```text
https://github.com/unicity-aos/aos-ce/.github/workflows/release.yml@refs/tags/<version>
```

The channel pointer is a strict TOML document with a channel name, monotonically
increasing generation, publication and expiry times, the immutable release
metadata digest, and the same four target records. Its exact accepted identity
is:

```text
https://github.com/unicity-aos/aos-ce/.github/workflows/promote-channel.yml@refs/heads/main
```

Product versions use `YYYY.MINOR.PATCH`: the year is calendar-based, while
minor and patch are canonical SemVer numbers rather than months.

The installer authenticates the channel first, authenticates and hashes the
referenced immutable release metadata second, then authenticates and hashes the
selected target archive. It stores each accepted pointer and bundle together in
an immutable generation directory under `~/.aos/update/channels/`, then
atomically activates that generation. An installation lock serializes product
replacement and channel acceptance. Inactive generation directories are safe
after an interrupted install. A lower generation, or different bytes at the
same generation, is rejected. A rollback is therefore a new, higher generation
that points to an older immutable release; channel history is never replaced.

Astrid Runtime 0.9.4 predates Astrid's immutable signed release metadata. Its
compatibility entry consequently records `release-metadata-available = false`
and empty source/asset/digest fields. It cannot be promoted by inventing those
values. A future runtime pin must name the signed Astrid metadata asset, source
commit, and BLAKE3 digest.

## Promotion operations

`.github/workflows/promote-channel.yml` is manual-only and must run from `main`.
It authenticates an already-published immutable AOS release, requires both
readiness gates, verifies that the tag resolves to the recorded source commit,
and requires a generation greater than the authenticated current pointer. The
workflow signs the new pointer before its publication job.

Create these GitHub environments with Joshua as the required reviewer and
prevent administrator bypass:

- `release`
- `aos-channel-stable`
- `aos-channel-dev`
- `aos-channel-nightly`

The YAML environment name alone is not an approval policy; repository
environment settings and tag rules are part of the release boundary. Protect
calendar-version tags from force updates and deletion. The channel publication
job retains immutable generation-named assets before replacing the signed
current pointer. Publishing its bundle first makes readers racing the two asset
updates fail closed.

No channel is created by merging this foundation. The current false readiness
flags continue to block release and promotion workflows.

The three signed channel identities are complete, but automated dev-candidate
and nightly build trains are not enabled by this change. Until those immutable
build contracts exist, all three channels advance only through the protected
promotion workflow.

Homebrew remains stable-only. Its formula updater should consume the signed
`stable` pointer, never `dev`, `nightly`, or an arbitrary published version. A
channel rollback protects new direct installs; an already-upgraded Homebrew
installation normally needs a forward patch release rather than a version
downgrade.
