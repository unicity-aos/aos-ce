#!/usr/bin/env python3
"""Qualify the exact Astrid source pinned by AOS before release packaging.

This source regression gate supplements artifact signature/archive validation;
it neither authenticates artifacts nor repairs an installed runtime.
"""
from __future__ import annotations

import argparse
from pathlib import Path
import re
import subprocess
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[1]
RECOVERY_FIX = "34bfab971cb780815b50ae9a51dc23d6646161d9"
TEST_MODULE = "engine::wasm::interruption_tests"
REQUIRED_TESTS = {
    "fuel_interruption_denies_instead_of_returning_a_skippable_error",
    "fuel_interruption_discards_mutated_guest_state",
    "fixed_size_pool_replaces_a_trapped_instance",
    "successful_calls_keep_the_warm_guest_instance",
    "epoch_interruption_denies_and_recovers",
    "guest_trap_denies_instead_of_skipping_the_guard",
    "cancelled_call_discards_mutated_guest_state",
    "fuel_interruption_halts_the_real_dispatcher_chain_then_recovers",
}


def git(source: Path, *args: str) -> str:
    return subprocess.check_output(["git", "-C", str(source), *args], text=True).strip()


def validate_source(source: Path, commit: str, version: str) -> None:
    if re.fullmatch(r"[0-9a-f]{40}", commit) is None:
        raise ValueError("runtime source commit must be a full lowercase Git commit")
    if git(source, "rev-parse", "HEAD") != commit:
        raise ValueError("Astrid checkout does not match the pinned runtime source commit")
    if git(source, "status", "--porcelain", "--untracked-files=no"):
        raise ValueError("Astrid source must be clean before recovery qualification")
    try:
        git(source, "merge-base", "--is-ancestor", RECOVERY_FIX, commit)
    except subprocess.CalledProcessError as error:
        raise ValueError("runtime source lacks Astrid #2010; do not release AOS with this runtime") from error
    with (source / "Cargo.toml").open("rb") as file:
        actual = tomllib.load(file)["workspace"]["package"]["version"]
    if actual != version:
        raise ValueError("Astrid workspace version does not match the pinned runtime version")


def validate_test_list(output: str) -> None:
    found = {line.removesuffix(": test").removeprefix(TEST_MODULE + "::")
             for line in output.splitlines()
             if line.startswith(TEST_MODULE + "::") and line.endswith(": test")}
    missing = REQUIRED_TESTS - found
    if missing:
        raise ValueError("runtime recovery regressions missing: " + ", ".join(sorted(missing)))


def validate_test_result(output: str) -> None:
    match = re.search(r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored", output)
    if match is None or int(match[1]) < len(REQUIRED_TESTS) or match.groups()[1:] != ("0", "0"):
        raise ValueError("runtime recovery suite did not pass all required regressions")


def qualify(source: Path, commit: str, version: str) -> None:
    validate_source(source, commit, version)
    command = ["cargo", "test", "--locked", "-p", "astrid-capsule", "--lib", TEST_MODULE]
    listing = subprocess.check_output(command + ["--", "--list"], cwd=source, text=True)
    validate_test_list(listing)
    result = subprocess.check_output(command, cwd=source, text=True)
    print(result, end="")
    validate_test_result(result)
    # Cargo must not have changed tracked sources, lockfiles or submodules.
    validate_source(source, commit, version)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("astrid_source", type=Path)
    parser.add_argument("--compatibility", type=Path, default=ROOT / "release/runtime-compatibility.toml")
    args = parser.parse_args()
    with args.compatibility.open("rb") as file:
        runtime = tomllib.load(file)["runtime"]
    if runtime["repository"] != "astrid-runtime/astrid" or not runtime["release-metadata-available"]:
        raise ValueError("recovery qualification requires the signed Astrid runtime metadata pin")
    qualify(args.astrid_source.resolve(), runtime["source-commit"], runtime["version"])
    print(f"Qualified runtime recovery: Astrid {runtime['version']} at {runtime['source-commit']}")


if __name__ == "__main__":
    try:
        main()
    except (KeyError, ValueError, subprocess.CalledProcessError) as error:
        print(f"runtime recovery: {error}", file=sys.stderr)
        raise SystemExit(1)
