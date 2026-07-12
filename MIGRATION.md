# Migration ledger

This ledger records the provenance of every repository imported into Unicity
AOS. Do not delete source repositories or rewrite source commit identities as
part of an import.

| Source repository | Destination | Final source commit | Release tags | License | Status |
| --- | --- | --- | --- | --- | --- |
| `unicity-astrid/astralis` | `distros/community` | pending | pending | pending | planned |
| `unicity-astrid/capsule-cli` | `capsules/capsule-cli` | `e1e180a62f24d4f210c79d8330d625b28b4de3ce` | `v0.2.0` | MIT OR Apache-2.0 | imported |
| `unicity-astrid/capsule-agents` | `capsules/capsule-agents` | `63b691e4e16e556b2363371f6e82e4a6ff3b7f5f` | pending | pending | imported |
| `unicity-astrid/capsule-context-engine` | `capsules/capsule-context-engine` | `6a9f6554fcd9989913763e530284d46bdcc938fa` | pending | pending | imported |
| `unicity-astrid/capsule-forge` | `capsules/capsule-forge` | `8dc54a134892f1ed798f1cade203fd54d89d5e0e` | pending | MIT OR Apache-2.0 | imported |
| `unicity-astrid/capsule-fs` | `capsules/capsule-fs` | `663e9b3ee7783f70654758031b860a32661edbbf` | pending | pending | imported |
| `unicity-astrid/capsule-hook-bridge` | `capsules/capsule-hook-bridge` | `274381de4687908561bec55552b344aabe4bb852` | pending | pending | imported |
| `unicity-astrid/capsule-http` | `capsules/capsule-http` | `bbe78a77d6bbdac7d1c10131f0a6bcedf364a379` | pending | pending | imported |
| `unicity-astrid/capsule-identity` | `capsules/capsule-identity` | `1364a437f30122558a70ad703acc23d48f144ee6` | pending | pending | imported |
| `unicity-astrid/capsule-memory` | `capsules/capsule-memory` | `ceab000a84d252ca63d20489126852a27918b16a` | pending | pending | imported |
| `unicity-astrid/capsule-openai` | `capsules/capsule-openai` | `9f564e92421137009f3061600c9e9fde1813e013` | pending | pending | imported |
| `unicity-astrid/capsule-openai-compat` | `capsules/capsule-openai-compat` | `2e08879c772cb88d66772f7c0c4802ac51ed7c3b` | pending | pending | imported |
| Remaining first-party `unicity-astrid/capsule-*` repositories | `capsules/<name>` | pending | pending | pending | planned |

Copied or local-only capsule directories require a source, license, and
ownership decision before import. The stale `capsule-anthropic` repository is
excluded and must not be revived.
