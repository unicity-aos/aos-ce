#!/usr/bin/env python3
"""Validate and stage numbered AOS dev release candidates."""

import argparse
from pathlib import Path

import nightly_version
import release_metadata


def validate(root: Path, version: str) -> None:
    if release_metadata.RELEASE_CANDIDATE.fullmatch(version) is None:
        raise ValueError("release candidate must be YYYY.MINOR.PATCH-rc.N with N positive")
    if version.split("-rc.", 1)[0] != nightly_version.canonical_base(root):
        raise ValueError("release candidate base must match the AOS source version")


def validate_tag(root: Path, version: str) -> None:
    release_metadata.validate_channel_version("dev", version)
    if release_metadata.RELEASE_CANDIDATE.fullmatch(version):
        validate(root, version)


def stage(root: Path, version: str) -> None:
    validate(root, version)
    date = nightly_version.table(root / "distros/community/unicity-ce/Distro.toml")["distro"]["release-date"]
    nightly_version.stage_product_version(root, version, date)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("validate", "validate-tag", "stage"))
    parser.add_argument("--root", type=Path, default=nightly_version.ROOT)
    parser.add_argument("--version", required=True)
    args = parser.parse_args()
    {"validate": validate, "validate-tag": validate_tag, "stage": stage}[args.command](args.root, args.version)
