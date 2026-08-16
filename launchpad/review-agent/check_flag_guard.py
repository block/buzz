"""Control for #137: --seed and --no-contain must be unusable outside a control run.

CONTAINMENT.md's mutation-seam section names both flags "controls only" and says
"No stage may pass either" — but until this control existed, nothing enforced that at
runtime. Any caller of the CLI (a future production wiring, a copy-pasted invocation)
could pass `--no-contain` and silently disable containment, or `--seed` and make the
boundary nonce predictable, with no signal that either happened.

The gate: both flags require REVIEW_AGENT_ALLOW_MUTATION=true in the environment.
Without it, contain.py's CLI must refuse to run rather than silently honouring the flag.
"""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

from contain import CONTROL_FLAGS_ENV_VAR

HERE = Path(__file__).parent
PAYLOAD = str(HERE / "fixtures" / "captured-pr.json")

failures: list[str] = []


def run(args: list[str], *, allow: bool) -> tuple[int, str, str]:
    env = dict(os.environ)
    if allow:
        env[CONTROL_FLAGS_ENV_VAR] = "true"
    else:
        env.pop(CONTROL_FLAGS_ENV_VAR, None)
    proc = subprocess.run(
        [sys.executable, str(HERE / "contain.py"), *args],
        capture_output=True,
        text=True,
        env=env,
    )
    return proc.returncode, proc.stdout, proc.stderr


def check(ok: bool, label: str) -> None:
    print(f"{'PASS' if ok else 'FAIL'}  {label}")
    if not ok:
        failures.append(label)


# --- without the env var, both flags must be refused ------------------------

code, out, err = run(["--payload", PAYLOAD, "--seed", "unguarded", "--json"], allow=False)
check(
    code != 0 and CONTROL_FLAGS_ENV_VAR in err,
    f"--seed without REVIEW_AGENT_ALLOW_MUTATION is refused (exit {code})",
)

code, out, err = run(["--payload", PAYLOAD, "--no-contain", "--json"], allow=False)
check(
    code != 0 and CONTROL_FLAGS_ENV_VAR in err,
    f"--no-contain without REVIEW_AGENT_ALLOW_MUTATION is refused (exit {code})",
)

# --- plain invocation (neither flag) must still work unguarded ---------------

code, out, err = run(["--payload", PAYLOAD, "--json"], allow=False)
check(code == 0, f"plain invocation needs no env var (exit {code}): {err[-300:]}")

# --- with the env var, both flags work exactly as before ---------------------

code, out, err = run(["--payload", PAYLOAD, "--seed", "guarded", "--json"], allow=True)
check(code == 0, f"--seed with REVIEW_AGENT_ALLOW_MUTATION=true still works (exit {code}): {err[-300:]}")

code, out, err = run(["--payload", PAYLOAD, "--no-contain", "--json"], allow=True)
check(
    code == 0 and '"nonce"' in out,
    f"--no-contain with REVIEW_AGENT_ALLOW_MUTATION=true still works (exit {code}): {err[-300:]}",
)

# --- contain() itself must refuse enabled=False directly, not only via the CLI --
# CONTAINMENT.md tells later stages (#117, #118) to call render()/contain() DIRECTLY,
# never through the CLI — a gate that only watched argv would leave those callers
# completely unprotected. This is the gap an independent review found in the first
# version of this fix.

from contain import CONTROL_FLAGS_ENV_VAR as _ENV_VAR
from contain import contain as _contain
from contain import make_nonce as _make_nonce

_saved_env = os.environ.pop(_ENV_VAR, None)
try:
    try:
        _contain("pr_body", "payload", _make_nonce("guard"), enabled=False)
        check(False, "contain(enabled=False) without the env var raises")
    except RuntimeError as exc:
        check(
            _ENV_VAR in str(exc),
            f"contain(enabled=False) without the env var names the env var (got: {exc})",
        )

    os.environ[_ENV_VAR] = "true"
    result = _contain("pr_body", "payload", _make_nonce("guard"), enabled=False)
    check(
        result.block == "payload",
        "contain(enabled=False) with the env var still disables containment",
    )
finally:
    if _saved_env is None:
        os.environ.pop(_ENV_VAR, None)
    else:
        os.environ[_ENV_VAR] = _saved_env

print(f"\n{len(failures)} failure(s)")
sys.exit(1 if failures else 0)
