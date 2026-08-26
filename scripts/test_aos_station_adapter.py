#!/usr/bin/env python3
"""Adversarial and fail-closed tests for the Station v2 adapter."""

from __future__ import annotations

import io
import json
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))

from aos_station_adapter import policy
from aos_station_adapter import adapter as adapter_module
from aos_station_adapter.adapter import (
    AdapterError,
    _record_summary,
    check_station_tools_pin,
    prepare_v2_records,
)
from aos_station_adapter.release import (
    VerificationError,
    _load_source_manifest,
    _verify_capsule,
    verify_current_main_drift,
    verify_release,
    verify_source_tree,
)


ROOT = Path(__file__).resolve().parent.parent
TAG_ROOT = (
    Path(os.environ["AOS_CE_TAG_SOURCE_ROOT"])
    if os.environ.get("AOS_CE_TAG_SOURCE_ROOT")
    else None
)
EVIDENCE_ASSETS = (
    Path(os.environ["AOS_CE_EVIDENCE_ASSETS"])
    if os.environ.get("AOS_CE_EVIDENCE_ASSETS")
    else None
)
STATION_TOOLS = (
    Path(os.environ["AOS_CE_STATION_TOOLS"])
    if os.environ.get("AOS_CE_STATION_TOOLS")
    else None
)
B3SUM = shutil.which("b3sum") or "b3sum"


def _copy_entry(source: str, destination: str) -> None:
    """Hard-link immutable fixture inputs; copy only when a filesystem forbids it."""
    try:
        os.link(source, destination)
    except OSError:
        shutil.copy2(source, destination)


def _replace_file(path: Path, content: str | bytes) -> None:
    # Fixtures are often hard-linked to the read-only evidence directory.
    path.unlink()
    if isinstance(content, bytes):
        path.write_bytes(content)
    else:
        path.write_text(content, encoding="utf-8")


class StationAdapterTests(unittest.TestCase):
    def _require_directory(
        self,
        value: Path | None,
        variable: str,
        message: str,
    ) -> Path:
        if value is None or not value.is_dir():
            if os.environ.get("GITHUB_ACTIONS"):
                self.fail(f"{variable} must be provisioned when GITHUB_ACTIONS is set")
            self.skipTest(message)
        return value

    def require_tag(self) -> Path:
        return self._require_directory(
            TAG_ROOT,
            "AOS_CE_TAG_SOURCE_ROOT",
            "set AOS_CE_TAG_SOURCE_ROOT to the detached 2026.1.3 checkout",
        )

    def require_assets(self) -> Path:
        return self._require_directory(
            EVIDENCE_ASSETS,
            "AOS_CE_EVIDENCE_ASSETS",
            "set AOS_CE_EVIDENCE_ASSETS to the release evidence assets",
        )

    def require_station_tools(self) -> Path:
        return self._require_directory(
            STATION_TOOLS,
            "AOS_CE_STATION_TOOLS",
            "set AOS_CE_STATION_TOOLS to the reviewed local Station checkout",
        )

    def clone_station_tools(self, root: Path) -> Path:
        source = self.require_station_tools()
        destination = root / "station-tools"
        cloned = subprocess.run(
            ["git", "clone", "--local", "--no-hardlinks", str(source), str(destination)],
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertEqual(cloned.returncode, 0, cloned.stderr)
        detached = subprocess.run(
            ["git", "-C", str(destination), "checkout", "--detach", policy.STATION_TOOLS_COMMIT],
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertEqual(detached.returncode, 0, detached.stderr)
        return destination

    def clone_assets(self, root: Path) -> Path:
        destination = root / "assets"
        shutil.copytree(self.require_assets(), destination, copy_function=_copy_entry)
        return destination

    def verify_fixture(self, assets: Path) -> dict[str, object]:
        return verify_release(
            assets,
            self.require_tag(),
            b3sum=B3SUM,
            skip_sigstore=True,
            current_main_root=ROOT,
        )

    def test_ci_inputs_are_required_when_running_in_github_actions(self) -> None:
        if not os.environ.get("GITHUB_ACTIONS"):
            return
        missing = [
            name
            for name, value in (
                ("AOS_CE_TAG_SOURCE_ROOT", TAG_ROOT),
                ("AOS_CE_EVIDENCE_ASSETS", EVIDENCE_ASSETS),
                ("AOS_CE_STATION_TOOLS", STATION_TOOLS),
            )
            if value is None
        ]
        self.assertEqual(missing, [], f"CI inputs were not provisioned: {missing}")

    def test_station_tools_clean_pin_is_accepted(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            checkout = self.clone_station_tools(Path(temporary))
            result = check_station_tools_pin(checkout)
            self.assertEqual(result["commit"], policy.STATION_TOOLS_COMMIT)
            self.assertEqual(result["tree"], policy.STATION_TOOLS_TREE)

    def test_station_tools_replacement_ref_rejects_before_bridge_build(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            checkout = self.clone_station_tools(root)
            tracked = "crates/station-publish/src/v2.rs"
            replacement = subprocess.run(
                ["git", "-C", str(checkout), "hash-object", "-w", "--stdin"],
                input=b"replacement object fixture\n",
                check=False,
                capture_output=True,
            )
            self.assertEqual(replacement.returncode, 0, replacement.stderr.decode())
            replacement_blob = replacement.stdout.decode("ascii").strip()
            self.assertEqual(len(replacement_blob), 40)

            for command in (
                ["git", "-C", str(checkout), "read-tree", policy.STATION_TOOLS_TREE],
                [
                    "git",
                    "-C",
                    str(checkout),
                    "update-index",
                    "--add",
                    "--cacheinfo",
                    f"100644,{replacement_blob},{tracked}",
                ],
            ):
                completed = subprocess.run(command, check=False, capture_output=True, text=True)
                self.assertEqual(completed.returncode, 0, completed.stderr)
            replacement_tree_result = subprocess.run(
                ["git", "-C", str(checkout), "write-tree"],
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(replacement_tree_result.returncode, 0, replacement_tree_result.stderr)
            replacement_tree = replacement_tree_result.stdout.strip()
            self.assertEqual(len(replacement_tree), 40)

            replaced = subprocess.run(
                [
                    "git",
                    "-C",
                    str(checkout),
                    "replace",
                    policy.STATION_TOOLS_TREE,
                    replacement_tree,
                ],
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(replaced.returncode, 0, replaced.stderr)
            for command in (
                ["git", "-C", str(checkout), "read-tree", "--reset", replacement_tree],
                ["git", "-C", str(checkout), "checkout-index", "--all", "--force"],
            ):
                completed = subprocess.run(command, check=False, capture_output=True, text=True)
                self.assertEqual(completed.returncode, 0, completed.stderr)

            porcelain = subprocess.run(
                [
                    "git",
                    "-C",
                    str(checkout),
                    "status",
                    "--porcelain=v1",
                    "--untracked-files=all",
                ],
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(porcelain.returncode, 0, porcelain.stderr)
            self.assertEqual(porcelain.stdout.strip(), "")
            self.assertEqual(
                subprocess.run(
                    ["git", "-C", str(checkout), "rev-parse", "HEAD^{tree}"],
                    check=False,
                    capture_output=True,
                    text=True,
                ).stdout.strip(),
                policy.STATION_TOOLS_TREE,
            )
            no_replace_porcelain = subprocess.run(
                [
                    "git",
                    "--no-replace-objects",
                    "-C",
                    str(checkout),
                    "status",
                    "--porcelain=v1",
                    "--untracked-files=all",
                ],
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(no_replace_porcelain.returncode, 0, no_replace_porcelain.stderr)
            self.assertIn(tracked, no_replace_porcelain.stdout)

            with self.assertRaisesRegex(AdapterError, "content drift"):
                check_station_tools_pin(checkout)
            with patch.object(adapter_module, "_bridge_manifest") as bridge:
                with self.assertRaisesRegex(AdapterError, "content drift"):
                    prepare_v2_records(
                        checkout,
                        root / "assets",
                        root / "source",
                        root / "submission",
                    )
                bridge.assert_not_called()

    def test_tagged_source_replacement_ref_rejects_before_bridge_build(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = self.require_tag()
            checkout = root / "source"
            cloned = subprocess.run(
                [
                    "git",
                    "clone",
                    "--local",
                    "--no-hardlinks",
                    str(source),
                    str(checkout),
                ],
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(cloned.returncode, 0, cloned.stderr)
            detached = subprocess.run(
                [
                    "git",
                    "-C",
                    str(checkout),
                    "checkout",
                    "--detach",
                    policy.SOURCE_COMMIT,
                ],
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(detached.returncode, 0, detached.stderr)

            tracked = "capsules/capsule-cli/Capsule.toml"
            original = subprocess.run(
                [
                    "git",
                    "--no-replace-objects",
                    "-C",
                    str(checkout),
                    "show",
                    f"{policy.SOURCE_TREE}:{tracked}",
                ],
                check=False,
                capture_output=True,
            )
            self.assertEqual(original.returncode, 0, original.stderr.decode())
            replacement = subprocess.run(
                [
                    "git",
                    "-C",
                    str(checkout),
                    "hash-object",
                    "-w",
                    "--stdin",
                ],
                input=original.stdout + b"\n# replacement object fixture\n",
                check=False,
                capture_output=True,
            )
            self.assertEqual(replacement.returncode, 0, replacement.stderr.decode())
            replacement_blob = replacement.stdout.decode("ascii").strip()
            self.assertEqual(len(replacement_blob), 40)

            for command in (
                ["git", "-C", str(checkout), "read-tree", policy.SOURCE_TREE],
                [
                    "git",
                    "-C",
                    str(checkout),
                    "update-index",
                    "--add",
                    "--cacheinfo",
                    f"100644,{replacement_blob},{tracked}",
                ],
            ):
                completed = subprocess.run(command, check=False, capture_output=True, text=True)
                self.assertEqual(completed.returncode, 0, completed.stderr)
            replacement_tree_result = subprocess.run(
                ["git", "-C", str(checkout), "write-tree"],
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(replacement_tree_result.returncode, 0, replacement_tree_result.stderr)
            replacement_tree = replacement_tree_result.stdout.strip()
            self.assertEqual(len(replacement_tree), 40)

            replaced = subprocess.run(
                [
                    "git",
                    "-C",
                    str(checkout),
                    "replace",
                    policy.SOURCE_TREE,
                    replacement_tree,
                ],
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(replaced.returncode, 0, replaced.stderr)
            for command in (
                ["git", "-C", str(checkout), "read-tree", "--reset", replacement_tree],
                ["git", "-C", str(checkout), "checkout-index", "--all", "--force"],
            ):
                completed = subprocess.run(command, check=False, capture_output=True, text=True)
                self.assertEqual(completed.returncode, 0, completed.stderr)

            porcelain = subprocess.run(
                [
                    "git",
                    "-C",
                    str(checkout),
                    "status",
                    "--porcelain=v1",
                    "--untracked-files=all",
                ],
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(porcelain.returncode, 0, porcelain.stderr)
            self.assertEqual(porcelain.stdout.strip(), "")
            default_head = subprocess.run(
                ["git", "-C", str(checkout), "rev-parse", "HEAD^{commit}"],
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(default_head.returncode, 0, default_head.stderr)
            self.assertEqual(default_head.stdout.strip(), policy.SOURCE_COMMIT)
            default_tree = subprocess.run(
                [
                    "git",
                    "-C",
                    str(checkout),
                    "rev-parse",
                    f"{policy.SOURCE_COMMIT}^{{tree}}",
                ],
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(default_tree.returncode, 0, default_tree.stderr)
            self.assertEqual(default_tree.stdout.strip(), policy.SOURCE_TREE)
            no_replace_porcelain = subprocess.run(
                [
                    "git",
                    "--no-replace-objects",
                    "-C",
                    str(checkout),
                    "status",
                    "--porcelain=v1",
                    "--untracked-files=all",
                ],
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(no_replace_porcelain.returncode, 0, no_replace_porcelain.stderr)
            self.assertIn(tracked, no_replace_porcelain.stdout)

            with patch.object(adapter_module, "_bridge_manifest") as bridge:
                assets = self.clone_assets(root)
                with self.assertRaisesRegex(VerificationError, "source checkout is dirty"):
                    verify_source_tree(checkout)
                with self.assertRaisesRegex(VerificationError, "source checkout is dirty"):
                    verify_release(
                        assets,
                        checkout,
                        b3sum=B3SUM,
                        skip_sigstore=True,
                        current_main_root=ROOT,
                    )
                bridge.assert_not_called()

    def test_station_tools_tracked_dirt_rejects_before_bridge_build(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            checkout = self.clone_station_tools(root)
            tracked = checkout / "Cargo.toml"
            tracked.write_text(tracked.read_text(encoding="utf-8") + "\n", encoding="utf-8")
            with patch.object(adapter_module, "_bridge_manifest") as bridge:
                with self.assertRaisesRegex(
                    AdapterError,
                    "(checkout is dirty|content drift).*",
                ):
                    prepare_v2_records(
                        checkout,
                        root / "assets",
                        root / "source",
                        root / "submission",
                    )
                bridge.assert_not_called()

    def test_station_tools_untracked_dirt_rejects_before_bridge_build(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            checkout = self.clone_station_tools(root)
            (checkout / "untracked-dirt.txt").write_text("dirty\n", encoding="utf-8")
            with patch.object(adapter_module, "_bridge_manifest") as bridge:
                with self.assertRaisesRegex(
                    AdapterError,
                    "(checkout is dirty|extra filesystem entry).*",
                ):
                    prepare_v2_records(
                        checkout,
                        root / "assets",
                        root / "source",
                        root / "submission",
                    )
                bridge.assert_not_called()

    def test_station_tools_station_publish_v2_content_drift_rejects_before_bridge_build(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            checkout = self.clone_station_tools(root)
            tracked = checkout / "crates" / "station-publish" / "src" / "v2.rs"
            tracked.write_text(tracked.read_text(encoding="utf-8") + "\n// drift\n", encoding="utf-8")
            with patch.object(adapter_module, "_bridge_manifest") as bridge:
                with self.assertRaisesRegex(
                    AdapterError,
                    "(checkout is dirty|content drift).*",
                ):
                    prepare_v2_records(
                        checkout,
                        root / "assets",
                        root / "source",
                        root / "submission",
                    )
                bridge.assert_not_called()

    def test_station_tools_ignored_extra_rejects_before_bridge_build(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            checkout = self.clone_station_tools(root)
            ignored = checkout / "ignored-extra.capsule"
            ignored.write_bytes(b"ignored\n")
            with patch.object(adapter_module, "_bridge_manifest") as bridge:
                with self.assertRaisesRegex(AdapterError, "extra filesystem entry.*ignored-extra.capsule"):
                    prepare_v2_records(
                        checkout,
                        root / "assets",
                        root / "source",
                        root / "submission",
                    )
                bridge.assert_not_called()

    def test_station_tools_symlink_and_mode_drift_reject_before_bridge_build(self) -> None:
        for mutation, expected in (("symlink", "unexpected symlink"), ("mode", "mode drift")):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                checkout = self.clone_station_tools(root)
                target = checkout / "Cargo.toml"
                if mutation == "symlink":
                    target.unlink()
                    target.symlink_to("README.md")
                else:
                    target.chmod(0o755)
                with patch.object(adapter_module, "_bridge_manifest") as bridge:
                    with self.assertRaisesRegex(AdapterError, expected):
                        prepare_v2_records(
                            checkout,
                            root / "assets",
                            root / "source",
                            root / "submission",
                        )
                    bridge.assert_not_called()

    def test_station_tools_index_optimization_flags_reject_before_bridge_build(self) -> None:
        for flag in ("assume", "skip"):
            with self.subTest(flag=flag), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                checkout = self.clone_station_tools(root)
                tracked = "crates/station-publish/src/v2.rs"
                command = ["git", "-C", str(checkout), "update-index"]
                command.append("--assume-unchanged" if flag == "assume" else "--skip-worktree")
                command.append(tracked)
                changed = subprocess.run(command, check=False, capture_output=True, text=True)
                self.assertEqual(changed.returncode, 0, changed.stderr)
                with patch.object(adapter_module, "_bridge_manifest") as bridge:
                    with self.assertRaisesRegex(AdapterError, "assume-unchanged or skip-worktree"):
                        prepare_v2_records(
                            checkout,
                            root / "assets",
                            root / "source",
                            root / "submission",
                        )
                    bridge.assert_not_called()

    def test_tagged_capsule_identity_matrix_uses_detached_tree(self) -> None:
        source = self.require_tag()
        self.assertEqual(
            verify_source_tree(source),
            {
                "commit": policy.SOURCE_COMMIT,
                "tree": "46a8a976d54eb82ac922c56ae41d713595895ff5",
                "tag": policy.SOURCE_TAG,
            },
        )
        identities: list[tuple[str, str]] = []
        for item in policy.SEED:
            with self.subTest(directory=item.directory):
                manifest, manifest_bytes = _load_source_manifest(source, item)
                package = manifest["package"]
                self.assertEqual(package["name"], item.package)
                self.assertEqual(package["version"], item.version)
                self.assertGreater(len(manifest_bytes), 0)
                identities.append((package["name"], package["version"]))
        self.assertEqual(len(identities), 19)
        self.assertEqual(len(set(identities)), 19)

    def test_tagged_capsule_archives_verify_as_canonical_nineteen(self) -> None:
        source = self.require_tag()
        assets = self.require_assets()
        verified = []
        for item in policy.SEED:
            with self.subTest(package=item.package):
                manifest, manifest_bytes = _load_source_manifest(source, item)
                verified.append(
                    _verify_capsule(assets / item.asset, item, manifest, manifest_bytes)
                )
        self.assertEqual(len(verified), 19)
        self.assertEqual(
            {entry["package"] for entry in verified},
            {item.package for item in policy.SEED},
        )

    def test_current_main_drift_is_separate_and_contains_three_blocked_additions(self) -> None:
        drift = verify_current_main_drift(ROOT)
        self.assertTrue(drift["separate_from_release_proof"])
        self.assertEqual(len(drift["seed_directories"]), 19)
        self.assertEqual(len(drift["allowlist"]), 22)
        self.assertEqual(
            set(drift["blocked_additions"]),
            {"capsule-mcp", "capsule-hook-adapter-oracle", "capsule-meta-harness"},
        )

    def test_default_live_path_exits_before_any_output_or_station_mutation(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            output = root / "output"
            result = subprocess.run(
                [
                    sys.executable,
                    str(ROOT / "scripts" / "aos_station_adapter.py"),
                    "--artifacts",
                    str(root / "missing-assets"),
                    "--source-root",
                    str(root / "missing-source"),
                    "--current-main-root",
                    str(root / "missing-main"),
                    "--station-tools",
                    str(root / "missing-station-tools"),
                    "--output",
                    str(output),
                ],
                cwd=ROOT,
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(result.returncode, 2)
            self.assertFalse(output.exists())
            self.assertIn("blocked before writes", result.stderr)
            self.assertNotIn("write_submission", result.stderr)

    def test_publish_option_is_permanently_disabled_before_any_output(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            output = root / "output"
            result = subprocess.run(
                [
                    sys.executable,
                    str(ROOT / "scripts" / "aos_station_adapter.py"),
                    "--artifacts",
                    str(root / "missing-assets"),
                    "--station-tools",
                    str(root / "missing-station-tools"),
                    "--output",
                    str(output),
                    "--publish",
                ],
                cwd=ROOT,
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(result.returncode, 2)
            self.assertFalse(output.exists())
            self.assertIn("blocked before writes", result.stderr)

    def test_failed_dry_run_does_not_follow_symlink_or_modify_nonempty_output(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            existing = root / "existing"
            existing.mkdir()
            sentinel = existing / "sentinel"
            sentinel.write_text("preserve", encoding="utf-8")
            args = [
                sys.executable,
                str(ROOT / "scripts" / "aos_station_adapter.py"),
                "--artifacts",
                str(root / "missing-assets"),
                "--source-root",
                str(root / "missing-source"),
                "--current-main-root",
                str(root / "missing-main"),
                "--station-tools",
                str(root / "missing-station-tools"),
                "--output",
                str(existing),
                "--dry-run",
            ]
            result = subprocess.run(args, cwd=ROOT, check=False, capture_output=True, text=True)
            self.assertEqual(result.returncode, 2)
            self.assertEqual(sentinel.read_text(encoding="utf-8"), "preserve")
            self.assertEqual(list(existing.iterdir()), [sentinel])

            linked = root / "linked"
            linked.symlink_to(existing, target_is_directory=True)
            linked_args = list(args)
            linked_args[-2] = str(linked)
            result = subprocess.run(
                linked_args,
                cwd=ROOT,
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(result.returncode, 2)
            self.assertEqual(list(existing.iterdir()), [sentinel])

    def test_dry_run_fixture_is_not_ready_and_has_only_typed_record_files(self) -> None:
        source = self.require_tag()
        assets = self.require_assets()
        station_tools = self.require_station_tools()
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "output"
            result = subprocess.run(
                [
                    sys.executable,
                    str(ROOT / "scripts" / "aos_station_adapter.py"),
                    "--artifacts",
                    str(assets),
                    "--source-root",
                    str(source),
                    "--current-main-root",
                    str(ROOT),
                    "--station-tools",
                    str(station_tools),
                    "--output",
                    str(output),
                    "--skip-sigstore",
                    "--dry-run",
                    "--json",
                ],
                cwd=ROOT,
                check=False,
                capture_output=True,
                text=True,
                timeout=180,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            report = json.loads(result.stdout)
            self.assertFalse(report["readiness"])
            self.assertEqual(report["publication"]["record_count"], 19)
            self.assertFalse(report["publication"]["fixture_readiness"])
            records = list((output / "submission" / "records").rglob("*.json"))
            artifacts = list((output / "submission" / "artifacts").rglob("*.capsule"))
            self.assertEqual(len(records), 19)
            self.assertEqual(len(artifacts), 19)
            publication_report = json.loads(
                (output / "reports" / "publication-v2.json").read_text(encoding="utf-8")
            )
            self.assertNotIn("records", publication_report)
            proposal = json.loads(
                (output / "submission" / "proposals" / "aos-ce-2026.1.3.json").read_text(
                    encoding="utf-8"
                )
            )
            self.assertEqual(len(proposal["allowlist_coordinates"]), 19)
            self.assertNotIn("package", proposal)

    def test_forbidden_authority_registry_checkpoint_and_event_head_tables_at_any_depth(self) -> None:
        item = policy.SEED[0]
        for key in ("authority", "registry", "checkpoint", "event-head"):
            with self.subTest(key=key):
                with tempfile.TemporaryDirectory() as temporary:
                    root = Path(temporary)
                    manifest = root / "capsules" / item.directory / "Capsule.toml"
                    manifest.parent.mkdir(parents=True)
                    manifest.write_text(
                        f'[package]\nname = "{item.package}"\nversion = "{item.version}"\n'
                        f"\n[{key}]\nmarker = \"unexpected\"\n",
                        encoding="utf-8",
                    )
                    with self.assertRaisesRegex(VerificationError, key):
                        _load_source_manifest(root, item)

                    nested = root / "nested" / "capsules" / item.directory / "Capsule.toml"
                    nested.parent.mkdir(parents=True)
                    nested.write_text(
                        f'[package]\nname = "{item.package}"\nversion = "{item.version}"\n'
                        f"\n[package.metadata]\n{key} = \"unexpected\"\n",
                        encoding="utf-8",
                    )
                    with self.assertRaisesRegex(VerificationError, key):
                        _load_source_manifest(root / "nested", item)

    def test_unknown_top_level_manifest_field_is_rejected(self) -> None:
        item = policy.SEED[0]
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest = root / "capsules" / item.directory / "Capsule.toml"
            manifest.parent.mkdir(parents=True)
            manifest.write_text(
                f'[package]\nname = "{item.package}"\nversion = "{item.version}"\n'
                '\n[future_contract]\nvalue = "unexpected"\n',
                encoding="utf-8",
            )
            with self.assertRaisesRegex(VerificationError, "unknown top-level"):
                _load_source_manifest(root, item)

    def test_manifest_name_and_version_mismatch_are_rejected(self) -> None:
        item = policy.SEED[0]
        for field, value in (("name", "not-aos"), ("version", "9.9.9")):
            with self.subTest(field=field):
                with tempfile.TemporaryDirectory() as temporary:
                    root = Path(temporary)
                    manifest = root / "capsules" / item.directory / "Capsule.toml"
                    manifest.parent.mkdir(parents=True)
                    name = value if field == "name" else item.package
                    version = value if field == "version" else item.version
                    manifest.write_text(
                        f'[package]\nname = "{name}"\nversion = "{version}"\n',
                        encoding="utf-8",
                    )
                    with self.assertRaisesRegex(VerificationError, "identity/version"):
                        _load_source_manifest(root, item)

    def test_wrong_tree_is_rejected_before_capsule_processing(self) -> None:
        source = self.require_tag()
        with patch.object(policy, "SOURCE_TREE", "0" * 40):
            with self.assertRaisesRegex(VerificationError, "source tree"):
                verify_source_tree(source)

    def test_wrong_tag_and_workflow_identity_are_rejected(self) -> None:
        self.require_tag()
        cases = (
            ("tag", 'tag = "2026.1.3"', 'tag = "2026.1.2"', "tag"),
            (
                "workflow",
                'release-workflow-identity = "https://github.com/unicity-aos/aos-ce/.github/workflows/release.yml@refs/tags/2026.1.3"',
                'release-workflow-identity = "https://example.invalid/workflow"',
                "workflow identity",
            ),
        )
        for field, old, new, message in cases:
            with self.subTest(field=field):
                with tempfile.TemporaryDirectory() as temporary:
                    assets = self.clone_assets(Path(temporary))
                    metadata = assets / "unicity-aos-2026.1.3-release.toml"
                    text = metadata.read_text(encoding="utf-8")
                    self.assertIn(old, text)
                    _replace_file(metadata, text.replace(old, new, 1))
                    with self.assertRaisesRegex(VerificationError, message):
                        self.verify_fixture(assets)

    def test_wrong_bundle_is_rejected_by_exact_asset_inventory(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            assets = self.clone_assets(Path(temporary))
            (assets / "aos-cli.capsule.sigstore.json").unlink()
            with self.assertRaisesRegex(VerificationError, "asset set differs"):
                self.verify_fixture(assets)

    def test_wrong_digest_is_rejected_by_executable_blake3_check(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            assets = self.clone_assets(Path(temporary))
            checksums = assets / "BLAKE3SUMS.txt"
            lines = checksums.read_text(encoding="utf-8").splitlines()
            for index, line in enumerate(lines):
                if line.endswith("  aos-cli.capsule"):
                    lines[index] = f"{'0' * 64}  aos-cli.capsule"
                    break
            else:  # pragma: no cover - fixture corruption
                self.fail("aos-cli checksum was not present")
            _replace_file(checksums, "\n".join(lines) + "\n")
            with self.assertRaisesRegex(VerificationError, "BLAKE3 mismatch"):
                self.verify_fixture(assets)

    def test_archive_embedded_manifest_bytes_must_match_tagged_source(self) -> None:
        source = self.require_tag()
        assets = self.require_assets()
        item = policy.SEED[0]
        manifest, manifest_bytes = _load_source_manifest(source, item)
        with tempfile.TemporaryDirectory() as temporary:
            changed = Path(temporary) / item.asset
            with tarfile.open(assets / item.asset, "r:gz") as reader, tarfile.open(
                changed, "w:gz"
            ) as writer:
                for member in reader.getmembers():
                    if member.isfile():
                        stream = reader.extractfile(member)
                        self.assertIsNotNone(stream)
                        payload = stream.read() if stream is not None else b""
                        if member.name == "Capsule.toml":
                            payload += b"\n# structural mismatch\n"
                        member.size = len(payload)
                        writer.addfile(member, io.BytesIO(payload))
                    else:
                        writer.addfile(member)
            with self.assertRaisesRegex(VerificationError, "bytes differ"):
                _verify_capsule(changed, item, manifest, manifest_bytes)

    def test_equivocation_classification_is_rejected_at_record_boundary(self) -> None:
        item = policy.SEED[0]
        value = {
            "record": "records/aos/aos-cli/0.2.0.json",
            "artifact": "artifacts/blake3/00/fixture.capsule",
            "coordinate": "@aos/aos-cli",
            "version": item.version,
            "publication_digest": "blake3:" + "0" * 64,
            "package_digest": "blake3:" + "1" * 64,
            "manifest_digest": "blake3:" + "2" * 64,
            "content_digest": "blake3:" + "3" * 64,
            "artifact_size": 1,
            "classification": "Equivocation",
            "readiness": False,
            "fixture": True,
            "schema": "aos-station-prepare-v2-result-v1",
        }
        with self.assertRaisesRegex(AdapterError, "unexpected publication classification"):
            _record_summary(value, item)

    def test_adapter_sources_have_no_v1_write_or_network_publish_surface(self) -> None:
        paths = [
            ROOT / "scripts" / "aos_station_adapter.py",
            *sorted((ROOT / "scripts" / "aos_station_adapter").glob("*.py")),
            ROOT / "scripts" / "aos_station_adapter" / "prepare_v2_bridge.rs",
        ]
        for path in paths:
            content = path.read_text(encoding="utf-8")
            self.assertNotIn("write_submission", content, path.as_posix())
            self.assertNotIn("git push", content, path.as_posix())
            self.assertNotIn("gh release upload", content, path.as_posix())

    def test_workflow_provisions_locked_graph_and_b3sum_only_failure_path(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "station-submit.yml").read_text(
            encoding="utf-8"
        )
        self.assertIn("CARGO_HOME", workflow)
        self.assertIn("cargo fetch", workflow)
        self.assertIn("--manifest-path \"$AOS_CE_STATION_TOOLS/Cargo.toml\"", workflow)
        self.assertIn("--offline --locked", workflow)
        self.assertIn("cargo install b3sum --locked --version 1.8.5", workflow)
        self.assertIn('b3sum_stderr="$RUNNER_TEMP/aos-b3sum-build.stderr"', workflow)
        self.assertIn('test "$status" -eq 101', workflow)
        self.assertIn(
            "grep -F 'no matching package named `thiserror` found' \"$b3sum_stderr\"",
            workflow,
        )
        self.assertIn(
            "grep -F 'offline mode (--offline)' \"$b3sum_stderr\"",
            workflow,
        )


if __name__ == "__main__":
    unittest.main(verbosity=2)
