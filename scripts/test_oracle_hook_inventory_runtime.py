#!/usr/bin/env python3
"""Exercise registered commands through real packaged capsules in a QA home.

Requires a marked disposable Distro containing aos-mcp, the adapter, and the
signed oracle-hook-test-policy fixture. Does not run client applications.
"""
import argparse
import json
import os
from pathlib import Path
import runpy
import subprocess
import time
import uuid


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--astrid", type=Path, required=True)
    parser.add_argument("--aos", type=Path, required=True)
    parser.add_argument("--oracle-root", type=Path, required=True)
    args = parser.parse_args()
    root = args.root.resolve(strict=True)
    if 'id = "oracle-bus-qa"' not in (root / "Distro.toml").read_text():
        raise RuntimeError("refusing an unmarked application home")
    oracle = args.oracle_root.resolve(strict=True)
    codec = runpy.run_path(str(oracle / "plugins/common/bin/aos-native-hook"))
    env = {key: value for key, value in os.environ.items()
           if not key.startswith(("AOS_", "ASTRID_", "GROK_", "CLAUDE_", "CODEX_", "PLUGIN_"))}
    env.update(AOS_HOME=str(root / "aos"), ASTRID_HOME=str(root / "aos/runtime"),
               ASTRID_RUN_DIR=str(root / "aos/run"), ASTRID_WORKSPACE_STATE_DIR=".aos",
               ASTRID_HOOK_TOKEN="d" * 64)

    def runtime(*argv):
        result = subprocess.run([str(args.astrid), *argv], cwd=root, env=env,
                                text=True, capture_output=True, timeout=45)
        if result.returncode:
            raise RuntimeError((argv, result.stdout, result.stderr))
        return result.stdout

    results = []
    for host, directory in (("codex", "unicity-aos"), ("claude", "claude"), ("grok", "grok")):
        principal = host + "-code"
        principals = json.loads(runtime("agent", "list", "--format", "json"))
        if not any(entry["principal"] == principal for entry in principals):
            runtime("agent", "create", principal, "--yes")
        # An install initializes this principal's background runs as well as
        # grants. Granting names alone is not a substitute for that lifecycle.
        for attempt in range(2):
            try:
                runtime("init", "--distro", str(root / "Distro.toml"), "--allow-unsigned",
                        "--target-principal", principal, "--grant-capsules", "--yes")
                break
            except RuntimeError as error:
                if attempt or "Rate limited: max 2 BeginCapsuleInstallBatch requests per minute" not in str(error):
                    raise
                print(json.dumps({"host": host, "fixture_setup_wait_seconds": 61,
                                  "reason": "production install-batch rate limit"}), flush=True)
                time.sleep(61)
        def source(name):
            return json.loads(runtime("capsule", "show", name, "--agent", principal,
                                      "--format", "json"))["source"]
        def config(name, key, value):
            runtime("capsule", "config", name, "--agent", principal, "--set", key + "=" + value)
        policy = "oracle-hook-test-policy"
        inventory = codec["hook_inventory"](host)
        registration = "test-policy=" + source(policy)
        scoped = {canonical: [registration] for _, canonical, mode in inventory.values()
                  if mode not in ("observe", "context", "worktree")}
        config("aos-mcp", "AOS_ORACLE_ADAPTER_SOURCE_ID", source("aos-hook-adapter-oracle"))
        config("aos-hook-adapter-oracle", "AOS_ORACLE_REQUIRED_HOOK_POLICIES", json.dumps(scoped))
        plugin = oracle / "plugins" / directory
        hook_env = dict(env, AOS_BIN=str(args.aos), AOS_PLUGIN_ROOT=str(plugin), PLUGIN_ROOT=str(plugin))
        hook_env[host.upper() + "_PLUGIN_ROOT"] = str(plugin)
        hooks = json.loads((plugin / "hooks/hooks.json").read_text())["hooks"]
        session = "inventory-" + uuid.uuid4().hex

        def invoke(event, denied=False, extra=None, context=False):
            name, canonical, mode = inventory[event]
            commands = [entry["command"] for group in hooks[name] for entry in group["hooks"]
                        if "aos-native-hook" in entry["command"]]
            assert len(commands) == 1, (host, event, commands)
            payload = {"session_id": session, "cwd": str(root), "prompt": "hello",
                       "tool_name": "Bash", "tool_input": {"command": "echo harmless"}}
            if host == "grok":
                payload["toolName"] = payload.pop("tool_name")
                payload["toolInput"] = payload.pop("tool_input")
            payload.update(extra or {})
            started = time.monotonic()
            result = subprocess.run(["/bin/sh", "-c", commands[0]], cwd=root, env=hook_env,
                input=json.dumps(payload), text=True, capture_output=True, timeout=12)
            assert "could not be delivered" not in result.stderr, (host, event, result.stderr)
            value = json.loads(result.stdout) if result.stdout.strip() else {}
            if denied and mode == "exit2":
                assert result.returncode == 2 and "test-policy-deny" in result.stderr, result
            else:
                assert result.returncode == 0, result
                expected = codec["native_output"](host, event, {
                    "schema_version": 1, "event": event,
                    "decision": {"skip": denied, "reason": "test-policy-deny"},
                    "context": "observed:" + canonical if context else None})
                assert value == expected, (host, event, value, expected, result.stderr)
            results.append({"host": host, "event": event, "canonical": canonical,
                            "denied": denied, "seconds": round(time.monotonic() - started, 3)})
            print(json.dumps(results[-1]), flush=True)

        config(policy, "QA_MODE", "allow")
        invoke("user_prompt_submit")  # Establish the authenticated session first.
        for event, (_, _, mode) in sorted(inventory.items()):
            if mode != "worktree" and event != "session_end":
                invoke(event)
        config(policy, "QA_MODE", "context")
        for event, (_, _, mode) in sorted(inventory.items()):
            if mode == "context":
                invoke(event, context=True)
        config(policy, "QA_MODE", "deny")
        for event, (_, _, mode) in sorted(inventory.items()):
            if mode not in ("worktree", "observe", "context"):
                invoke(event, denied=True)
        # The recursive Stop callback must not be forced into another turn.
        invoke("stop", extra={"stop_hook_active": True, "stopHookActive": True})
        if host == "grok":
            invoke("session_end", extra={"subagentType": "worker"})
            invoke("pre_tool_use", denied=True)  # Parent route remains authenticated.
        invoke("session_end")
    print(json.dumps({"passed": len(results), "boundary": "registered commands + packaged bus, not live clients"}))


if __name__ == "__main__":
    main()
