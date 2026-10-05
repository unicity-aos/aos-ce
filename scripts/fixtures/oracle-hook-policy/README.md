# Packaged Oracle hook test policy

This is a test-only signed capsule, not a distribution component. Build it with
`astrid capsule build`, using a disposable `ASTRID_HOME` signing directory and
the shared host Cargo caches. Do not install it in a real user's runtime.

Prepare a disposable workspace containing a Distro with `id = "oracle-bus-qa"`
and local packaged `aos-mcp` (role `uplink`), `aos-hook-adapter-oracle`, and this
policy. Create its application runtime directory before starting the daemon;
set `ASTRID_HOME`, `ASTRID_RUN_DIR`, and `ASTRID_WORKSPACE_STATE_DIR=.aos` to that
workspace only. The distro may be unsigned under explicit QA trust; capsule
signatures and the production source/principal verification are not bypassed.

Run `scripts/test_oracle_hook_inventory_runtime.py --root <workspace> --astrid
<runtime-binary> --aos <bootstrap-binary> --oracle-root <Oracles-checkout>`.
The test provisions three actual principals and exercises their registered
native commands through the packaged broker, adapter, and responder. It checks
no-objection, each supported denial format, optional context, recursive Stop,
and Grok child teardown followed by a still-authenticated parent tool request.
It respects the production install-batch rate limit during fixture setup.

Stop the explicitly selected QA daemon afterward using the same application
variables. Retain logs on failure. A passing run proves the registered command
and packaged bus boundary, not execution inside Claude/Codex/Grok applications,
a protected administrator deployment, or a release certification.

The AOS mapping fixture is exported from Oracles' authoritative codec:

```sh
python3 scripts/sync-hook-registrations.py --check --export-contract \
  <AOS>/capsules/capsule-hook-adapter-oracle/testdata/native-hook-inventory.json
```

Run that command from Oracles; AOS's unit test verifies every exported native
event against the actual canonical mapping and response class.
