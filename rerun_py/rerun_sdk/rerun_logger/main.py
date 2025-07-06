"""Main entry point for rerun-logger."""

from __future__ import annotations

import os
import subprocess
import sys


def exe_suffix() -> str:
    if sys.platform.startswith("win"):
        return ".exe"
    return ""


def add_exe_suffix(path: str) -> str:
    if not path.endswith(exe_suffix()):
        return path + exe_suffix()
    return path


def main() -> int:
    """Run the rerun-logger binary."""
    if "RERUN_LOGGER_PATH" in os.environ:
        print(f"Using overridden RERUN_LOGGER_PATH={os.environ['RERUN_LOGGER_PATH']}", file=sys.stderr)
        target_path = os.environ["RERUN_LOGGER_PATH"]
    else:
        # Look for rerun-logger binary in the rerun_cli directory
        target_path = os.path.join(os.path.dirname(__file__), "..", "rerun_cli", "rerun-logger")

    target_path = add_exe_suffix(target_path)

    if not os.path.exists(target_path):
        print(f"Error: Could not find rerun-logger binary at {target_path}", file=sys.stderr)
        return 1

    return subprocess.call([target_path, *sys.argv[1:]])


if __name__ == "__main__":
    sys.exit(main())