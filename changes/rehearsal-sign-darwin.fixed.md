- Added a dispatch-only, prepare-only Darwin rehearsal workflow that builds hosted
  artifacts, composes the 22-capsule Community bundle, and signs with an
  ephemeral QA key while keeping rehearsal evidence out of publication channels.
- Rehearsal AOS compiles from disposable overlays that bind runtime-compatibility,
  Distro astrid-version, QA pubkey, and ASTRID_RUNTIME_VERSION to 2026.9.0 without
  changing production source.
- After signing, overlay-built GNU AOS executes a fail-before-runtime Distro Apply
  against the signed Distro members in a disposable AOS_HOME, proving
  verify_selected_release accepts 2026.9.0 without starting runtime or FSKit.
