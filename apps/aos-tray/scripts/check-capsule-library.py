#!/usr/bin/env python3
"""Opt-in integration probe against an already-provisioned disposable AOS home.

Never installs capsules or operates on the default application home.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile

parser = argparse.ArgumentParser()
parser.add_argument("--aos", required=True, type=Path)
parser.add_argument("--astrid", required=True, type=Path)
parser.add_argument("--disposable-home", required=True, type=Path)
parser.add_argument("--expect-capsule", action="append", required=True)
args = parser.parse_args()
home = args.disposable_home.resolve(strict=True)
if home == Path.home() / ".aos" or not str(home).startswith("/private/tmp/"):
    raise SystemExit("This probe only accepts an explicit existing /private/tmp disposable home")
if sorted(p.name for p in (home / "runtime").iterdir()) != ["astrid.volume"]:
    raise SystemExit("Disposable runtime must begin stopped with only astrid.volume")
env = dict(os.environ, AOS_HOME=str(home), ASTRID_HOME=str(home / "runtime"),
           ASTRID_RUN_DIR=str(home / "run"), ASTRID_WORKSPACE_STATE_DIR=".aos",
           ASTRID_ENFORCED_DISTRO=str(home / "distributions/unicity-ce/Distro.toml"))

def run(binary, *arguments):
    # A persistent daemon must not inherit subprocess capture pipes.
    with tempfile.TemporaryFile() as stdout, tempfile.TemporaryFile() as stderr:
        result = subprocess.run([str(binary.resolve(strict=True)), *arguments], env=env,
                                cwd=home.parent, stdout=stdout, stderr=stderr, timeout=60)
        stdout.seek(0)
        stderr.seek(0)
        output, errors = stdout.read().decode(), stderr.read().decode()
        if result.returncode:
            raise RuntimeError(f"{arguments}: rc={result.returncode}: {output} {errors}")
        return output

def status():
    return json.loads(run(args.aos, "status", "--json", "--include-capsules", "--principal=default"))

before = status()
assert before["state"] == "stopped", before
assert before["capsule_inventory"]["state"] == "stopped", before
assert "capsules" not in before["capsule_inventory"], before
try:
    run(args.astrid, "start")
    active = status()
    assert active["state"] == "running", active
    library = active["capsule_inventory"]
    assert library["principal"] == "default", library
    assert library["state"] == "available", library
    names = [entry["name"] for entry in library["capsules"]]
    assert all(name in names for name in args.expect_capsule), names
    assert all(set(entry) == {"name", "version", "description"} for entry in library["capsules"])
finally:
    run(args.astrid, "stop")
after = status()
assert after["state"] == "stopped", after
assert after["capsule_inventory"]["state"] == "stopped", after
assert sorted(p.name for p in (home / "runtime").iterdir()) == ["astrid.volume"]
print(json.dumps({"result": "PASS", "before": before, "running": active, "after": after}, indent=2))
