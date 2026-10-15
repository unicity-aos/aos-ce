# AOS CE Station seed

`aos-seed.toml` is the reviewed policy projection for the 19 capsule packages
released by the exact `2026.1.3` AOS Community Edition tag. Each coordinate is
the literal `@aos/<package.name>@<package.version>` value from `Capsule.toml`;
the package version, not the product version, is the record version.

The adapter verifies the tag-bound release metadata, checksums, capsule
manifests, package versions, and Sigstore workflow identity before producing a
local proposal tree. It blocks the three current-main extras (`aos-mcp`,
`aos-hook-adapter-oracle`, and `aos-meta-harness`) until a new signed AOS
release. `aos-openai`, `aos-telegram`, and legacy `astrid-capsule-*` identities
are excluded from this seed.

The emitted dry-run tree contains typed `PublicationRecordV2` records and
content-addressed artifacts prepared by the reviewed `station-publish`
`prepare_v2` API. They carry a documented local fixture publisher only:
readiness stays false while the admitting-owner binding is unresolved. The
adapter never pushes, signs, or publishes Station content; network publication
is a permanently rejected option. The default/live path exits before writes.
