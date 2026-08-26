"""Pinned AOS Community Edition to Station policy.

The values in this module mirror ``release/station/aos-seed.toml``.  Keeping
the policy in code as well as in the reader-facing file lets the adapter fail
closed before parsing mutable current-main release lists.
"""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path


# This is the reviewed local v2 interface supplied to the adapter.  It is not
# a public Station repository/configuration pin; publication remains disabled.
STATION_TOOLS_COMMIT = "fd233f91b40d00c5f721ba99436ce30eca9036ba"
STATION_TOOLS_TREE = "0bf775f225bfb9177d5788ec8c32d5453f75c660"
PRODUCT_VERSION = "2026.1.3"
SOURCE_COMMIT = "efaa39f3f0ba80cf2c24985fd4383d8d73dd801a"
SOURCE_TREE = "46a8a976d54eb82ac922c56ae41d713595895ff5"
SOURCE_TAG = "2026.1.3"
SOURCE_REPOSITORY = "https://github.com/unicity-aos/aos-ce"
SOURCE_REPOSITORY_SLUG = "unicity-aos/aos-ce"
SOURCE_OWNER_ID = 261740341
SOURCE_REPOSITORY_ID = 1298696137
SOURCE_WORKFLOW_IDENTITY = (
    "https://github.com/unicity-aos/aos-ce/.github/workflows/release.yml"
    "@refs/tags/2026.1.3"
)
SIGSTORE_ISSUER = "https://token.actions.githubusercontent.com"
STATION_ID = "aos"
STATION_BASE_URL = "https://unicity-aos.github.io/station/v1"
RESERVED_NAMESPACE = "aos"
BLOCKED_DIRECTORIES = {
    "capsule-mcp": "aos-mcp",
    "capsule-hook-adapter-oracle": "aos-hook-adapter-oracle",
    "capsule-meta-harness": "aos-meta-harness",
}


@dataclass(frozen=True)
class SeedPackage:
    directory: str
    package: str
    version: str

    @property
    def coordinate(self) -> str:
        return f"@{RESERVED_NAMESPACE}/{self.package}@{self.version}"

    @property
    def asset(self) -> str:
        return f"{self.package}.capsule"


SEED: tuple[SeedPackage, ...] = (
    SeedPackage("capsule-cli", "aos-cli", "0.2.0"),
    SeedPackage("capsule-registry", "aos-registry", "0.2.0"),
    SeedPackage("capsule-openai-compat", "aos-openai-compat", "0.2.0"),
    SeedPackage("capsule-react", "aos-react", "0.2.2"),
    SeedPackage("capsule-session", "aos-session", "0.2.0"),
    SeedPackage("capsule-identity", "aos-identity", "0.2.0"),
    SeedPackage("capsule-users", "aos-users", "0.1.0"),
    SeedPackage("capsule-router", "aos-router", "0.2.0"),
    SeedPackage("capsule-prompt-builder", "aos-prompt-builder", "0.2.0"),
    SeedPackage("capsule-context-engine", "aos-context-engine", "0.2.0"),
    SeedPackage("capsule-hook-bridge", "aos-hook-bridge", "0.2.0"),
    SeedPackage("capsule-shell", "aos-shell", "0.2.0"),
    SeedPackage("capsule-http", "aos-http", "0.1.1"),
    SeedPackage("capsule-fs", "aos-fs", "0.2.0"),
    SeedPackage("capsule-system", "aos-system", "0.2.0"),
    SeedPackage("capsule-forge", "aos-forge", "0.1.0"),
    SeedPackage("capsule-skills", "aos-skills", "0.2.0"),
    SeedPackage("capsule-agents", "aos-agents", "0.2.0"),
    SeedPackage("capsule-memory", "aos-memory", "0.2.0"),
)

SEED_BY_ASSET = {item.asset: item for item in SEED}
SEED_BY_PACKAGE = {item.package: item for item in SEED}


def mapping_path(root: Path) -> Path:
    return root / "release" / "station" / "aos-seed.toml"
