#!/usr/bin/env python3
"""Run real Wry command cancellation checks against an isolated synthetic library."""
import json
from pathlib import Path
import subprocess
import sys


def main():
    if sys.platform != "darwin":
        raise SystemExit("This smoke check requires macOS with a GUI session.")
    root = Path(__file__).resolve().parents[1]
    subprocess.run(["cargo", "build", "-p", "kiroku-desktop", "--example", "native_job_smoke"], cwd=root, check=True)
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--no-deps", "--format-version=1"], cwd=root))
    binary = Path(metadata["target_directory"]) / "debug/examples/native_job_smoke"
    try:
        result = subprocess.run([str(binary)], cwd=root, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=90)
    except subprocess.TimeoutExpired as error:
        print((error.stdout or b"").decode(errors="replace"), end="", flush=True)
        raise
    print(result.stdout, end="", flush=True)
    assert result.returncode == 0, result.returncode
    assert "panicked" not in result.stdout
    for case in ("busy_18_commands", "cancel_before_apply", "early_cancel_6_commands", "optional_request_id", "partial_apply", "rollback", "catalog_early_cancel", "catalog_partial_rollback", "media_early_cancel", "media_hash_cancel"):
        assert f"PASS {case} " in result.stdout, case


if __name__ == "__main__":
    main()
