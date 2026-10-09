#!/usr/bin/env python3
"""Prepare an inert, deterministic AOS CE 2026.1.3 Station v1 submission.

The default command is deliberately fail-closed.  An explicit ``--dry-run``
executes the reviewed ``astrid-station-publish::prepare_v2`` API with a
non-admitting fixture identity, emits 19 sealed publication records and artifacts, and
round-trips them through the Station protocol. It never signs, pushes, or
publishes anything.
"""

from __future__ import annotations

import argparse
import json
import shutil
import sys
from pathlib import Path
from typing import Any

from aos_station_adapter import policy
from aos_station_adapter.adapter import (
    AdapterError,
    InertOnlyError,
    _tree_manifest,
    emit_submission_skeleton,
    prepare_v2_records,
)
from aos_station_adapter.release import VerificationError, verify_release


ROOT = Path(__file__).resolve().parent.parent


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    root.add_argument(
        "--artifacts",
        type=Path,
        required=True,
        help="local directory containing the exact signed 2026.1.3 release assets",
    )
    root.add_argument(
        "--source-root",
        type=Path,
        default=ROOT,
        help="clean detached exact 2026.1.3 source checkout",
    )
    root.add_argument(
        "--current-main-root",
        type=Path,
        default=ROOT,
        help="current-main checkout used only for the independent 22-vs-19 drift check",
    )
    root.add_argument(
        "--station-tools",
        type=Path,
        required=True,
        help=f"station-tools checkout at reviewed publication interface {policy.STATION_TOOLS_COMMIT}",
    )
    root.add_argument("--output", type=Path, required=True, help="fresh local output tree")
    root.add_argument("--b3sum", default="b3sum", help="pinned b3sum executable")
    root.add_argument("--cosign", default="cosign", help="cosign executable")
    root.add_argument(
        "--skip-sigstore",
        action="store_true",
        help="skip cryptographic cosign calls for offline fixture inspection (report remains incomplete)",
    )
    root.add_argument(
        "--dry-run",
        action="store_true",
        help="emit a deterministic local publication fixture; readiness and live authority remain false",
    )
    root.add_argument(
        "--publish",
        action="store_true",
        help="disabled permanently; network publication is outside this adapter",
    )
    root.add_argument("--json", action="store_true", help="print the final report as JSON")
    return root


def _write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True, ensure_ascii=True) + "\n", encoding="utf-8")


def _blocked_report(output: Path, error: str, *, verification: dict[str, Any] | None = None) -> dict[str, Any]:
    report = {
        "schema": "aos-station-adapter-report-v2",
        "readiness": False,
        "blockers": ["verification-failed"],
        "dry_run": True,
        "network_publish": False,
        "error": error,
        "record_count": 0,
        "event_count": 0,
    }
    if verification is not None:
        report["verification"] = verification
    # Validation failures may point at caller-owned paths. Never follow a
    # symlink or append a report to a non-empty tree that the adapter did not
    # create itself.
    if output.is_symlink():
        return report
    if output.exists() and (not output.is_dir() or any(output.iterdir())):
        return report
    output.mkdir(parents=True, exist_ok=True)
    _write_json(output / "reports" / "adapter-report.json", report)
    return report


def run(args: argparse.Namespace) -> dict[str, Any]:
    if args.publish:
        raise InertOnlyError(
            "network publication is permanently disabled; remove --publish and review the inert local tree"
        )
    if not args.dry_run:
        raise InertOnlyError(
            "admitting owner material is unresolved; default mode exits before any output; use --dry-run for inert inspection"
        )
    if args.output.exists() and (args.output.is_symlink() or not args.output.is_dir()):
        raise AdapterError(f"output path is not a directory: {args.output}")
    if args.output.exists() and any(args.output.iterdir()):
        raise AdapterError(f"output path must be fresh or empty: {args.output}")

    verification = verify_release(
        args.artifacts,
        args.source_root,
        b3sum=args.b3sum,
        cosign=args.cosign,
        skip_sigstore=args.skip_sigstore,
        current_main_root=args.current_main_root,
    )
    skeleton = emit_submission_skeleton(args.output, verification)
    publication = prepare_v2_records(
        args.station_tools,
        args.artifacts,
        args.source_root,
        skeleton["submission"],
        b3sum=args.b3sum,
    )
    repeat_submission = args.output / ".submission-repeat"
    repeat_submission.mkdir()
    repeat_publication = prepare_v2_records(
        args.station_tools,
        args.artifacts,
        args.source_root,
        repeat_submission,
        b3sum=args.b3sum,
    )
    if repeat_publication["records"] != publication["records"]:
        raise AdapterError("prepare_v2 output is not deterministic")
    if _tree_manifest(skeleton["submission"] / "records") != _tree_manifest(
        repeat_submission / "records"
    ) or _tree_manifest(skeleton["submission"] / "artifacts") != _tree_manifest(
        repeat_submission / "artifacts"
    ):
        raise AdapterError("prepare_v2 record/artifact bytes are not deterministic")
    shutil.rmtree(repeat_submission)
    # The record files are the sole per-package contract. Keep this report to
    # aggregate execution facts so it cannot become a second 19-record schema.
    publication_report = {
        "tool": publication["tool"],
        "interface": publication["interface"],
        "fixture": publication["fixture"],
        "record_count": publication["record_count"],
        "event_count": publication["event_count"],
    }
    _write_json(
        skeleton["reports"] / "publication-v2.json",
        publication_report,
    )
    _write_json(
        skeleton["reports"] / "readiness.json",
        {
            "schema": "aos-station-readiness-v2",
            "readiness": False,
            "blockers": ["admission-contract-pending", "owner-binding-unresolved"],
            "record_count": publication["record_count"],
            "event_count": publication["event_count"],
            "publication_schema": "station-v2",
        },
    )
    report = {
        "schema": "aos-station-adapter-report-v2",
        "readiness": False,
        "blockers": ["admission-contract-pending", "owner-binding-unresolved"],
        "dry_run": args.dry_run,
        "network_publish": False,
        "verification": verification,
        "submission": {
            "path": str(skeleton["submission"].relative_to(args.output)),
            "proposal_count": skeleton["proposal_count"],
            "record_count": publication["record_count"],
            "event_count": publication["event_count"],
        },
        "publication": {
            "schema": "station-v2",
            "interface": publication["interface"],
            "record_count": publication["record_count"],
            "event_count": publication["event_count"],
            "fixture_readiness": publication["fixture"]["readiness"],
            "fixture_binding": publication["fixture"]["binding"],
        },
        "consumer": {
            "interface": "astrid-station-protocol::PublicationRecordV2",
            "record_paths": publication["record_count"],
            "artifact_paths": publication["record_count"],
            "deterministic": True,
        },
    }
    _write_json(args.output / "reports" / "adapter-report.json", report)
    return report


def main(argv: list[str] | None = None) -> int:
    args = parser().parse_args(argv)
    try:
        report = run(args)
    except InertOnlyError as error:
        print(f"aos station adapter: blocked before writes: {error}", file=sys.stderr)
        return 2
    except (AdapterError, VerificationError, OSError, ValueError) as error:
        blocked = _blocked_report(args.output, str(error))
        if args.json:
            print(json.dumps(blocked, indent=2, sort_keys=True))
        else:
            print(f"aos station adapter: blocked: {error}", file=sys.stderr)
        return 2
    if args.json:
        print(json.dumps(report, indent=2, sort_keys=True))
    else:
        print(
            f"proposal-incomplete: verified {report['verification']['seed_count']} capsules; "
            f"prepared {report['publication']['record_count']} Station publication records with a non-admitting fixture; "
            "network publication disabled"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
