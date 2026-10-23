#!/usr/bin/env python3
"""Exercise the real macOS/Wry shutdown paths using synthetic in-memory jobs only."""
import json
from pathlib import Path
import re
import subprocess
import sys


def main():
    if sys.platform != "darwin":
        raise SystemExit("This smoke check requires macOS with a GUI session.")
    root = Path(__file__).resolve().parents[1]
    subprocess.run(["cargo", "build", "-p", "kiroku-desktop", "--example", "native_shutdown_smoke"], cwd=root, check=True)
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--no-deps", "--format-version=1"], cwd=root))
    binary = Path(metadata["target_directory"]) / "debug/examples/native_shutdown_smoke"
    for mode in (sys.argv[1:] or ("close", "native", "combined", "exit", "idle", "fast")):
        try:
            result = subprocess.run([str(binary), mode], cwd=root, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=20)
        except subprocess.TimeoutExpired as error:
            print((error.stdout or b"").decode(errors="replace"), end="", flush=True)
            raise
        print(result.stdout, end="", flush=True)
        expected = 23 if mode == "exit" else 0
        assert result.returncode == expected, (mode, result.returncode)
        assert "job_running=true" not in result.stdout, mode
        assert "panicked" not in result.stdout, mode
        assert re.search(r"SMOKE exited .*job_running=false", result.stdout), mode
        if mode != "idle":
            assert result.stdout.index("commit_finished") < result.stdout.index("SMOKE exited"), mode
        if mode not in ("idle", "fast"):
            assert "alive_after_6s cancelled=true new_job=rejected window=visible" in result.stdout, mode
        if mode in ("close", "combined"):
            assert "close_requested count=2" in result.stdout, mode
        if mode in ("native", "combined"):
            assert result.stdout.count("SMOKE native_terminate") == 2, mode
        print(f"PASS {mode}", flush=True)


if __name__ == "__main__":
    main()
