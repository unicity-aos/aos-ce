#!/usr/bin/env python3
"""Validate the immutable Linux musl metadata extension for an AOS release."""

from __future__ import annotations

import argparse
import hashlib
import sys
from pathlib import Path
from typing import Any

import release_metadata


KIND = "aos-release-musl-extension"
MUSL_TARGETS = (
    "aarch64-unknown-linux-musl",
    "x86_64-unknown-linux-musl",
)
ROOT_KEYS = {
    "schema-version",
    "kind",
    "product",
    "repository",
    "version",
    "tag",
    "source-commit",
    "release-workflow-identity",
    "legacy-release",
    "runtime-musl",
    "targets",
}
LEGACY_KEYS = {"metadata-asset", "metadata-sha256"}
RUNTIME_KEYS = {
    "repository",
    "version",
    "tag",
    "source-commit",
    "release-workflow-identity",
    "legacy-release-metadata-asset",
    "legacy-release-metadata-blake3",
    "musl-release-metadata-asset",
    "musl-release-metadata-blake3",
}
TARGET_KEYS = {"asset", "sha256", "blake3", "sigstore-bundle", "size"}


def validate_runtime_pin(value: Any, *, require_ready: bool) -> dict[str, Any]:
    """Validate a staged musl runtime pin and return its runtime table."""
    root = release_metadata.exact_keys(
        value, {"schema-version", "runtime"}, "musl runtime compatibility"
    )
    release_metadata.require(
        type(root["schema-version"]) is int and root["schema-version"] == 1,
        "musl runtime compatibility schema-version must be integer 1",
    )
    runtime = release_metadata.exact_keys(
        root["runtime"],
        RUNTIME_KEYS | {"release-ready"},
        "musl runtime compatibility.runtime",
    )
    release_ready = runtime["release-ready"]
    release_metadata.require(
        type(release_ready) is bool,
        "musl runtime compatibility release-ready must be a boolean",
    )
    if require_ready:
        release_metadata.require(
            release_ready,
            "musl runtime compatibility release-ready gate is false",
        )

    repository = release_metadata.string(
        runtime["repository"], "musl runtime repository"
    )
    release_metadata.require(
        repository == "astrid-runtime/astrid",
        "musl runtime repository must be astrid-runtime/astrid",
    )
    version = release_metadata.string(runtime["version"], "musl runtime version")
    release_metadata.require(
        release_metadata.SEMVER.fullmatch(version) is not None,
        "musl runtime version must be canonical semver",
    )
    tag = release_metadata.string(runtime["tag"], "musl runtime tag")
    release_metadata.require(tag == f"v{version}", "musl runtime tag/version mismatch")
    source_commit = release_metadata.string(
        runtime["source-commit"], "musl runtime source-commit"
    )
    release_metadata.require(
        release_metadata.COMMIT.fullmatch(source_commit) is not None,
        "musl runtime source commit is malformed",
    )
    identity = release_metadata.string(
        runtime["release-workflow-identity"],
        "musl runtime release-workflow-identity",
    )
    expected_identity = (
        "https://github.com/astrid-runtime/astrid/.github/workflows/"
        f"release.yml@refs/tags/v{version}"
    )
    release_metadata.require(
        identity == expected_identity,
        "musl runtime workflow identity is not the exact tag identity",
    )

    legacy_asset = release_metadata.string(
        runtime["legacy-release-metadata-asset"],
        "musl runtime legacy metadata asset",
    )
    release_metadata.require(
        legacy_asset == f"astrid-{version}-release.toml",
        "musl runtime legacy metadata asset is not canonical",
    )
    legacy_blake3 = release_metadata.string(
        runtime["legacy-release-metadata-blake3"],
        "musl runtime legacy metadata BLAKE3",
    )
    release_metadata.require(
        release_metadata.HEX_64.fullmatch(legacy_blake3) is not None,
        "musl runtime legacy metadata BLAKE3 is malformed",
    )

    musl_asset = runtime["musl-release-metadata-asset"]
    musl_blake3 = runtime["musl-release-metadata-blake3"]
    if release_ready:
        musl_asset = release_metadata.string(
            musl_asset, "musl runtime extension metadata asset"
        )
        release_metadata.require(
            musl_asset == f"astrid-{version}-musl-release.toml",
            "musl runtime extension metadata asset is not canonical",
        )
        musl_blake3 = release_metadata.string(
            musl_blake3, "musl runtime extension metadata BLAKE3"
        )
        release_metadata.require(
            release_metadata.HEX_64.fullmatch(musl_blake3) is not None,
            "musl runtime extension metadata BLAKE3 is malformed",
        )
    else:
        release_metadata.require(
            musl_asset == musl_blake3 == "",
            "unready musl runtime extension fields must remain empty",
        )
    return runtime


def validate_target_table(
    value: Any, *, version: str, context: str
) -> dict[str, dict[str, Any]]:
    """Validate the fixed two-target musl archive table."""
    table = release_metadata.exact_keys(value, set(MUSL_TARGETS), context)
    for target in MUSL_TARGETS:
        item = release_metadata.exact_keys(
            table[target], TARGET_KEYS, f"{context}.{target}"
        )
        asset = release_metadata.string(item["asset"], f"{context}.{target}.asset")
        expected_asset = f"unicity-aos-{version}-{target}.tar.gz"
        release_metadata.require(
            asset == expected_asset,
            f"{context}.{target}.asset must be {expected_asset}",
        )
        sigstore_bundle = release_metadata.string(
            item["sigstore-bundle"], f"{context}.{target}.sigstore-bundle"
        )
        release_metadata.require(
            sigstore_bundle == f"{asset}.sigstore.json",
            f"{context}.{target}.sigstore-bundle must name the asset bundle",
        )
        for algorithm in ("sha256", "blake3"):
            digest = release_metadata.string(
                item[algorithm], f"{context}.{target}.{algorithm}"
            )
            release_metadata.require(
                release_metadata.HEX_64.fullmatch(digest) is not None,
                f"{context}.{target}.{algorithm} is malformed",
            )
        release_metadata.require(
            type(item["size"]) is int and item["size"] > 0,
            f"{context}.{target}.size must be positive",
        )
    return table


def validate_extension(
    value: Any,
    *,
    legacy: dict[str, Any] | None = None,
    legacy_bytes: bytes | None = None,
) -> dict[str, Any]:
    """Validate an extension, optionally binding it to a legacy release."""
    release_metadata.require(
        (legacy is None) == (legacy_bytes is None),
        "legacy release binding requires both metadata and its bytes",
    )
    root = release_metadata.exact_keys(value, ROOT_KEYS, "musl release metadata")
    release_metadata.require(
        type(root["schema-version"]) is int and root["schema-version"] == 1,
        "musl release metadata schema-version must be integer 1",
    )
    kind = release_metadata.string(root["kind"], "musl release metadata kind")
    release_metadata.require(kind == KIND, f"musl release metadata kind must be {KIND}")
    product = release_metadata.string(
        root["product"], "musl release metadata product"
    )
    release_metadata.require(
        product == release_metadata.PRODUCT,
        f"musl release metadata product must be {release_metadata.PRODUCT}",
    )
    repository = release_metadata.string(
        root["repository"], "musl release metadata repository"
    )
    release_metadata.require(
        repository == release_metadata.REPOSITORY,
        f"musl release metadata repository must be {release_metadata.REPOSITORY}",
    )
    version = release_metadata.string(
        root["version"], "musl release metadata version"
    )
    release_metadata.require(
        release_metadata.VERSION.fullmatch(version) is not None,
        "musl release metadata version must be calendar semver",
    )
    tag = release_metadata.string(root["tag"], "musl release metadata tag")
    release_metadata.require(
        tag == version, "musl release metadata tag must equal version"
    )
    source_commit = release_metadata.string(
        root["source-commit"], "musl release metadata source-commit"
    )
    release_metadata.require(
        release_metadata.COMMIT.fullmatch(source_commit) is not None,
        "musl release metadata source-commit is malformed",
    )
    if release_metadata.is_nightly_version(version):
        release_metadata.require(
            release_metadata.nightly_source_commit(version) == source_commit,
            "musl release metadata nightly version must embed its source commit",
        )
    identity = release_metadata.string(
        root["release-workflow-identity"],
        "musl release metadata release-workflow-identity",
    )
    release_metadata.require(
        identity == release_metadata.release_workflow_identity(version, tag),
        "musl release metadata workflow identity is not the exact tag identity",
    )

    legacy_link = release_metadata.exact_keys(
        root["legacy-release"], LEGACY_KEYS, "musl release metadata.legacy-release"
    )
    legacy_asset = release_metadata.string(
        legacy_link["metadata-asset"],
        "musl release metadata legacy metadata asset",
    )
    release_metadata.require(
        legacy_asset == f"unicity-aos-{version}-release.toml",
        "musl release metadata legacy asset is not canonical",
    )
    legacy_digest = release_metadata.string(
        legacy_link["metadata-sha256"],
        "musl release metadata legacy metadata SHA-256",
    )
    release_metadata.require(
        release_metadata.HEX_64.fullmatch(legacy_digest) is not None,
        "musl release metadata legacy SHA-256 is malformed",
    )

    runtime_musl = release_metadata.exact_keys(
        root["runtime-musl"], RUNTIME_KEYS, "musl release metadata.runtime-musl"
    )
    validate_runtime_pin(
        {"schema-version": 1, "runtime": {**runtime_musl, "release-ready": True}},
        require_ready=True,
    )
    validate_target_table(
        root["targets"],
        version=version,
        context="musl release metadata.targets",
    )

    if legacy is not None:
        assert legacy_bytes is not None
        validated_legacy = release_metadata.validate_release(legacy)
        for key in (
            "product",
            "version",
            "tag",
            "source-commit",
            "release-workflow-identity",
        ):
            release_metadata.require(
                root[key] == validated_legacy[key],
                f"musl release metadata {key} differs from the legacy release",
            )
        release_metadata.require(
            legacy_digest == hashlib.sha256(legacy_bytes).hexdigest(),
            "musl release metadata does not bind the authenticated legacy release",
        )
    return root


def render_extension(value: dict[str, Any]) -> str:
    """Render an already-valid extension fixture as canonical TOML."""
    root = validate_extension(value)
    quote = release_metadata.quoted
    lines = [
        "schema-version = 1",
        f"kind = {quote(root['kind'])}",
        f"product = {quote(root['product'])}",
        f"repository = {quote(root['repository'])}",
        f"version = {quote(root['version'])}",
        f"tag = {quote(root['tag'])}",
        f"source-commit = {quote(root['source-commit'])}",
        f"release-workflow-identity = {quote(root['release-workflow-identity'])}",
        "",
        "[legacy-release]",
        f"metadata-asset = {quote(root['legacy-release']['metadata-asset'])}",
        f"metadata-sha256 = {quote(root['legacy-release']['metadata-sha256'])}",
        "",
        "[runtime-musl]",
    ]
    for key in (
        "repository",
        "version",
        "tag",
        "source-commit",
        "release-workflow-identity",
        "legacy-release-metadata-asset",
        "legacy-release-metadata-blake3",
        "musl-release-metadata-asset",
        "musl-release-metadata-blake3",
    ):
        lines.append(f"{key} = {quote(root['runtime-musl'][key])}")
    for target in MUSL_TARGETS:
        item = root["targets"][target]
        lines.extend(
            [
                "",
                f"[targets.{target}]",
                f"asset = {quote(item['asset'])}",
                f"sha256 = {quote(item['sha256'])}",
                f"blake3 = {quote(item['blake3'])}",
                f"sigstore-bundle = {quote(item['sigstore-bundle'])}",
                f"size = {item['size']}",
            ]
        )
    return "\n".join(lines) + "\n"


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    commands = root.add_subparsers(dest="command", required=True)
    validate = commands.add_parser("validate")
    validate.add_argument("path", type=Path)
    validate.add_argument(
        "--legacy-release",
        type=Path,
        help="optionally bind the extension to a legacy release metadata file",
    )
    return root


def main(argv: list[str] | None = None) -> int:
    args = parser().parse_args(argv)
    extension = release_metadata.load(args.path)
    if args.legacy_release is None:
        validate_extension(extension)
    else:
        legacy_bytes = args.legacy_release.read_bytes()
        validate_extension(
            extension,
            legacy=release_metadata.load(args.legacy_release),
            legacy_bytes=legacy_bytes,
        )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (
        KeyError,
        OSError,
        release_metadata.tomllib.TOMLDecodeError,
        TypeError,
        ValueError,
    ) as error:
        print(f"musl release metadata: {error}", file=sys.stderr)
        raise SystemExit(1)
