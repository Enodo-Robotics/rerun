"""See `python3 -m rerun_sdk.rerun_logger --help`."""

from __future__ import annotations

import sys

from .main import main

if __name__ == "__main__":
    sys.exit(main())