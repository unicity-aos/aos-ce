- Added a dispatch-only, prepare-only Darwin rehearsal workflow that builds hosted
  artifacts, composes the 22-capsule Community bundle, and signs with an
  ephemeral QA key while keeping rehearsal evidence out of publication channels.
- Rehearsal AOS compiles from disposable overlays that bind runtime-compatibility,
  Distro astrid-version, QA pubkey, and ASTRID_RUNTIME_VERSION to 2026.9.0 without
  changing production source.
