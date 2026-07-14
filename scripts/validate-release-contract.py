#!/usr/bin/env python3
"""Validate that every independently published AOS compatibility pin agrees."""

from __future__ import annotations

import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


def strings(path: str) -> dict[tuple[str, str], str]:
    """Read the string values used by the release contract's TOML files."""
    values: dict[tuple[str, str], str] = {}
    section = ""
    for line in (ROOT / path).read_text(encoding="utf-8").splitlines():
        section_match = re.match(r"\s*\[([^]]+)]\s*$", line)
        if section_match:
            section = section_match.group(1)
            continue
        value_match = re.match(r'\s*([A-Za-z0-9_-]+)\s*=\s*"([^"]*)"', line)
        if value_match:
            values[(section, value_match.group(1))] = value_match.group(2)
    return values


def workspace_dependency(values: dict[tuple[str, str], str], name: str) -> str:
    direct = values.get(("workspace.dependencies", name))
    if direct is not None:
        return direct
    text = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    match = re.search(
        rf'^\s*{re.escape(name)}\s*=\s*\{{[^\n]*\bversion\s*=\s*"([^"]+)"',
        text,
        re.MULTILINE,
    )
    if match:
        return match.group(1)
    raise ValueError(f"{name} must have an exact workspace version")


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def main() -> int:
    workspace = strings("Cargo.toml")
    product = strings("crates/unicity-aos-bootstrap/Cargo.toml")
    distro = strings("distros/community/unicity-ce/Distro.toml")
    compatibility = strings("release/runtime-compatibility.toml")

    product_version = product[("package", "version")]
    runtime_version = compatibility[("runtime", "version")]
    sdk_version = compatibility[("contracts", "sdk-rust-version")]

    require(
        re.fullmatch(r"20[0-9]{2}\.[0-9]+\.[0-9]+", product_version) is not None,
        "product version must be calendar semver (YYYY.MINOR.PATCH)",
    )
    require(
        compatibility[("product", "version")] == product_version,
        "runtime-compatibility product version does not match the AOS crate",
    )
    require(
        distro[("distro", "version")] == product_version,
        "distro version does not match the AOS crate",
    )
    require(
        compatibility[("runtime", "tag")] == f"v{runtime_version}",
        "runtime tag does not match the pinned runtime version",
    )
    require(
        compatibility[("runtime", "version-requirement")] == f"={runtime_version}",
        "runtime version requirement must be an exact pin",
    )
    require(
        distro[("distro", "astrid-version")] == f"={runtime_version}",
        "distro Astrid requirement does not match the bundled runtime",
    )
    for dependency in ("astrid-core", "astrid-uplink"):
        require(
            workspace_dependency(workspace, dependency) == f"={runtime_version}",
            f"workspace {dependency} must exactly pin the bundled runtime",
        )
    require(
        workspace_dependency(workspace, "astrid-sdk") == f"={sdk_version}",
        "workspace astrid-sdk must exactly pin compatibility metadata",
    )
    require(
        re.fullmatch(r"[0-9a-f]{40}", compatibility[("contracts", "commit")])
        is not None,
        "WIT compatibility commit must be a full lowercase Git commit",
    )
    require(
        re.fullmatch(
            r"[0-9a-f]{40}", compatibility[("contracts", "sdk-rust-commit")]
        )
        is not None,
        "SDK compatibility commit must be a full lowercase Git commit",
    )
    identity = compatibility[("runtime", "release-workflow-identity")]
    require(
        re.fullmatch(
            rf"https://github\.com/[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+/\.github/workflows/release\.yml@refs/tags/v{re.escape(runtime_version)}",
            identity,
        )
        is not None,
        "runtime Sigstore identity must be a GitHub release workflow at the pinned tag",
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (KeyError, TypeError, ValueError) as error:
        print(f"release contract: {error}", file=sys.stderr)
        raise SystemExit(1)
