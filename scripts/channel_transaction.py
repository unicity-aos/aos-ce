#!/usr/bin/env python3
"""Pack and unpack one atomic AOS channel-generation transaction."""

from __future__ import annotations

import argparse
import base64
import binascii
import json
import os
import re
import sys
from pathlib import Path
from typing import Any


CHANNELS = ("stable", "dev", "nightly")
MAX_GENERATION = 999_999_999_999_999_999
MAX_POINTER_BYTES = 1024 * 1024
MAX_BUNDLE_BYTES = 8 * 1024 * 1024
MAX_TRANSACTION_BYTES = 16 * 1024 * 1024


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def regular_bytes(path: Path, maximum: int, context: str) -> bytes:
    require(path.is_file() and not path.is_symlink(), f"{context} must be a regular file")
    size = path.stat().st_size
    require(0 < size <= maximum, f"{context} has an invalid size")
    return path.read_bytes()


def validate_channel(channel: str) -> str:
    require(channel in CHANNELS, "channel must be stable, dev, or nightly")
    return channel


def validate_generation(generation: int) -> int:
    require(
        type(generation) is int and 0 < generation <= MAX_GENERATION,
        f"generation must be between 1 and {MAX_GENERATION}",
    )
    return generation


def encode(value: bytes) -> str:
    return base64.b64encode(value).decode("ascii")


def decode(value: Any, maximum: int, context: str) -> bytes:
    require(isinstance(value, str) and value != "", f"{context} must be non-empty base64")
    require(re.fullmatch(r"[A-Za-z0-9+/]*={0,2}", value) is not None, f"{context} is malformed")
    try:
        decoded = base64.b64decode(value, validate=True)
    except (binascii.Error, ValueError) as error:
        raise ValueError(f"{context} is malformed") from error
    require(0 < len(decoded) <= maximum, f"{context} has an invalid decoded size")
    return decoded


def pack(args: argparse.Namespace) -> None:
    channel = validate_channel(args.channel)
    generation = validate_generation(args.generation)
    require(
        not args.output.exists() and not args.output.is_symlink(),
        "transaction output already exists",
    )
    transaction = {
        "schema-version": 1,
        "kind": "aos-channel-transaction",
        "channel": channel,
        "generation": generation,
        "channel-toml-base64": encode(
            regular_bytes(args.pointer, MAX_POINTER_BYTES, "channel pointer")
        ),
        "sigstore-bundle-base64": encode(
            regular_bytes(args.bundle, MAX_BUNDLE_BYTES, "Sigstore bundle")
        ),
    }
    encoded = (json.dumps(transaction, sort_keys=True, separators=(",", ":")) + "\n").encode()
    require(len(encoded) <= MAX_TRANSACTION_BYTES, "transaction is too large")
    args.output.write_bytes(encoded)


def load_transaction(path: Path) -> dict[str, Any]:
    payload = regular_bytes(path, MAX_TRANSACTION_BYTES, "transaction")
    try:
        value = json.loads(payload)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError("transaction is not valid JSON") from error
    require(isinstance(value, dict), "transaction must be a JSON object")
    expected = {
        "schema-version",
        "kind",
        "channel",
        "generation",
        "channel-toml-base64",
        "sigstore-bundle-base64",
    }
    require(set(value) == expected, "transaction fields do not match the exact schema")
    require(type(value["schema-version"]) is int and value["schema-version"] == 1, "transaction schema-version must be integer 1")
    require(value["kind"] == "aos-channel-transaction", "transaction kind is invalid")
    validate_channel(value["channel"])
    validate_generation(value["generation"])
    return value


def unpack(args: argparse.Namespace) -> None:
    transaction = load_transaction(args.input)
    require(transaction["channel"] == args.channel, "transaction channel does not match")
    require(transaction["generation"] == args.generation, "transaction generation does not match")
    require(args.output_dir.is_dir() and not args.output_dir.is_symlink(), "output directory must be a real directory")
    require(not any(args.output_dir.iterdir()), "output directory must be empty")
    pointer = decode(
        transaction["channel-toml-base64"], MAX_POINTER_BYTES, "channel-toml-base64"
    )
    bundle = decode(
        transaction["sigstore-bundle-base64"], MAX_BUNDLE_BYTES, "sigstore-bundle-base64"
    )
    for name, payload in (
        ("channel.toml", pointer),
        ("channel.toml.sigstore.json", bundle),
    ):
        descriptor = os.open(
            args.output_dir / name,
            os.O_WRONLY | os.O_CREAT | os.O_EXCL,
            0o600,
        )
        with os.fdopen(descriptor, "wb") as output:
            output.write(payload)
            output.flush()
            os.fsync(output.fileno())


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    commands = root.add_subparsers(dest="command", required=True)

    pack_command = commands.add_parser("pack")
    pack_command.add_argument("--channel", choices=CHANNELS, required=True)
    pack_command.add_argument("--generation", type=int, required=True)
    pack_command.add_argument("--pointer", type=Path, required=True)
    pack_command.add_argument("--bundle", type=Path, required=True)
    pack_command.add_argument("--output", type=Path, required=True)
    pack_command.set_defaults(run=pack)

    unpack_command = commands.add_parser("unpack")
    unpack_command.add_argument("--channel", choices=CHANNELS, required=True)
    unpack_command.add_argument("--generation", type=int, required=True)
    unpack_command.add_argument("--input", type=Path, required=True)
    unpack_command.add_argument("--output-dir", type=Path, required=True)
    unpack_command.set_defaults(run=unpack)
    return root


def main(argv: list[str] | None = None) -> int:
    args = parser().parse_args(argv)
    args.run(args)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError) as error:
        print(f"channel transaction: {error}", file=sys.stderr)
        raise SystemExit(1)
