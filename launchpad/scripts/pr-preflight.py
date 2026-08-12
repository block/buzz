#!/usr/bin/env python3
"""Deterministic pre-flight for the PR review agent — launchpad-26/buzz#116.

    python3 launchpad/scripts/pr-preflight.py 86
    python3 launchpad/scripts/pr-preflight.py --help

Emits the facts a review needs as JSON on stdout and reaches no conclusion about
the code. Everything lives in ``preflight_fetch`` and ``preflight_core``, which
are importable under those names; this file is only the entry point, because a
module named ``pr-preflight`` cannot be imported and a CLI no control can drive
is a CLI no control can make fail.
"""

from __future__ import annotations

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from preflight_fetch import main  # noqa: E402  (after the sys.path insert, deliberately)

if __name__ == "__main__":
    sys.exit(main())
