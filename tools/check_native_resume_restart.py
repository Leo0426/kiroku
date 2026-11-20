#!/usr/bin/env python3
"""Persist an interrupted neutral batch, exit, then recover through production UI."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
from check_native_bridge import require_unlocked_session


def main():
    require_unlocked_session()
    root = Path(__file__).resolve().parents[1]
    subprocess.run(["pnpm", "build"], cwd=root / "apps/desktop", check=True)
    subprocess.run(["cargo", "build", "-p", "kiroku-desktop", "--example", "native_resume_restart", "--features", "tauri/custom-protocol"], cwd=root, check=True)
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--no-deps", "--format-version=1"], cwd=root))
    binary = Path(metadata["target_directory"]) / "debug/examples/native_resume_restart"
    env = os.environ.copy()
    for name in ("http_proxy", "https_proxy", "all_proxy", "HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"):
        env.pop(name, None)
    with tempfile.TemporaryDirectory(prefix="kiroku-restart-check-") as fixture:
        for phase, expected in (("--prepare", "restart_fixture_persisted"), ("--run", "restart_production_ui_resume_undo")):
            result = subprocess.run([str(binary), phase, fixture], env=env, cwd=root, text=True,
                                    stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60)
            print(result.stdout, end="", flush=True)
            assert result.returncode == 0 and f"PASS {expected}" in result.stdout, phase


if __name__ == "__main__":
    main()
