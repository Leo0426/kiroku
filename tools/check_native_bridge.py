#!/usr/bin/env python3
"""Exercise actual frontend modules in WebKit with the production Tauri bridge."""
import json
import os
import plistlib
from pathlib import Path
import subprocess
import sys
import tempfile


def require_unlocked_session():
    state = plistlib.loads(subprocess.check_output(["/usr/sbin/ioreg", "-a", "-l", "-d", "1"]))
    nodes = [state] if isinstance(state, dict) else state
    sessions = [session for node in nodes for session in node.get("IOConsoleUsers", [])
                if session.get("kCGSSessionOnConsoleKey")]
    if not sessions or any(session.get("CGSSessionScreenIsLocked") for session in sessions):
        raise SystemExit("WebKit probe requires an unlocked macOS console session; unlock the screen and retry.")


def main():
    if sys.platform != "darwin":
        raise SystemExit("This smoke check requires macOS with a GUI session.")
    require_unlocked_session()
    root = Path(__file__).resolve().parents[1]
    subprocess.run(["pnpm", "exec", "tsc", "-p", "tests/native-bridge/tsconfig.json"], cwd=root / "apps/desktop", check=True)
    with tempfile.TemporaryDirectory(prefix="kiroku-bridge-assets-") as assets:
        subprocess.run(["node", str(root / "apps/desktop/tests/native-bridge/build.mjs"), assets], cwd=root, check=True)
        subprocess.run(["cargo", "build", "-p", "kiroku-desktop", "--example", "native_job_smoke"], cwd=root, check=True)
        metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--no-deps", "--format-version=1"], cwd=root))
        binary = Path(metadata["target_directory"]) / "debug/examples/native_job_smoke"
        try:
            env = os.environ.copy()
            # The child may only reach its explicit loopback source fixture.
            # Avoid user proxy environment settings forwarding that request.
            for key in ("http_proxy", "https_proxy", "all_proxy", "HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"):
                env.pop(key, None)
            env["NO_PROXY"] = env["no_proxy"] = "127.0.0.1,localhost,::1"
            env["XDG_CONFIG_HOME"] = str(Path(assets) / ".config")
            result = subprocess.run([str(binary), "--webview", assets], cwd=root, env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60)
        except subprocess.TimeoutExpired as error:
            print((error.stdout or b"").decode(errors="replace"), end="", flush=True)
            raise
        print(result.stdout, end="", flush=True)
        assert result.returncode == 0, result.returncode
        assert "panicked" not in result.stdout
        for case in ("before_listener", "delayed_started", "busy_and_settled", "real_preview_cancel", "real_export", "real_rollback", "network_batch_cancel_retry", "network_detail_cancel_retry", "import_component_roundtrip", "rebuild_cancel_transaction", "rebuild_retry_restore"):
            assert f"PASS bridge_{case}" in result.stdout, case


if __name__ == "__main__":
    main()
