"""Disposable visual fixture; never connects to Astrid or saves a grant."""
import json
from pathlib import Path
import socket
import subprocess
import tempfile
import time

app = Path(__file__).resolve().parent.parent / ".build/AOS Preview.app/Contents/MacOS/aos-tray"
with tempfile.TemporaryDirectory(prefix="aos-ui-", dir="/private/tmp") as root:
    endpoint = str(Path(root) / "ui.sock")
    process = subprocess.Popen([str(app), "--socket", endpoint])
    try:
        for _ in range(100):
            if Path(endpoint).exists():
                break
            if process.poll() is not None:
                raise RuntimeError("Preview exited before opening its socket")
            time.sleep(0.1)
        with socket.socket(socket.AF_UNIX) as client:
            client.settimeout(305)
            client.connect(endpoint)
            request = {
                "version": 1, "id": "visual-fixture", "timeoutSeconds": 300,
                "message": "A capsule tool is requesting capability approval.\n\nAction: tray-mcp-probe\nResource: tray-mcp-probe isolated\nReason: Capsule 'approval-tray-probe' requests approval\n\nApprove this request?",
                "options": [{"label": label} for label in
                            ["Approve Once", "Approve for Session", "Always Approve", "Deny"]],
                "consent": {
                    "version": 1, "kind": "action_approval", "action": "tray-mcp-probe",
                    "resource": "tray-mcp-probe isolated",
                    "reason": "Capsule 'approval-tray-probe' requests approval",
                    "lifetimes": ["none", "session", "durable", "none"],
                },
            }
            client.sendall(json.dumps(request).encode() + b"\n")
            print("VISUAL FIXTURE ONLY: awaiting selection", flush=True)
            print(client.recv(16384).decode(), flush=True)
    finally:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
