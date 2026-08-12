"""The 35-case control suite: every payload through every entry point.

Each control asserts the payload appears **only inside a data block**, never in a
position a reader would take as instruction. That assertion rests on the envelope, so it
fails the moment containment is removed — which is what step 9 measures.

Cases whose payload carries an unambiguous tell additionally assert a finding. The rest
are enumerated by id as the model layer's responsibility (#117). Naming the gap is the
point: a suite that silently covered 28 of 35 while reporting "all pass" would be the
coverage theatre this issue exists to prevent.
"""

from __future__ import annotations

from dataclasses import dataclass

from contain import ENTRY_POINTS, TOKEN, make_nonce, render, unescape
from corpus import Case, matrix
from detect import detect
from fetch import Surface


@dataclass
class Result:
    case_id: str
    contained: bool  # payload never appears outside a data block
    expected_finding: bool  # DECLARED in the fixture — never computed from detect()
    got_finding: bool  # a finding for this entry point reached render()
    detector_agrees: bool  # detect() alone matches the declared expectation
    detail: str = ""

    @property
    def passed(self) -> bool:
        return (
            self.contained
            and self.detector_agrees
            and (self.got_finding or not self.expected_finding)
        )


def _surfaces_for(case: Case) -> dict[str, Surface]:
    """The payload on its own surface, every other surface fetched-and-empty."""
    return {
        ep: Surface(ep, "ok", text=case.payload.text) if ep == case.entry_point
        else Surface(ep, "empty")
        for ep in ENTRY_POINTS
    }


def run_case(case: Case, *, enabled: bool = True) -> Result:
    nonce = make_nonce(f"suite-{case.id}")
    document, findings, _ = render(_surfaces_for(case), nonce, enabled=enabled)

    contained, why = _is_contained(document, case, nonce)

    # The expectation is DECLARED, not measured. It used to read
    # ``expected = bool(detect(...))`` — the same production detect() that produces
    # ``got`` via render() — so both sides of the assertion came from the code under
    # test. With detect() neutered to return nothing, every case's expectation became
    # False, ``got_finding or not expected_finding`` was trivially true, and this suite
    # reported 35/35 green while covering nothing. That is the coverage theatre the
    # module docstring claims to prevent, so the oracle now lives in the fixture.
    expected = case.payload.expected_detection
    got = any(f.entry_point == case.entry_point for f in findings)
    # Pinned separately, because ``got`` is satisfied by a delimiter finding too: the
    # breakout payload would still report a finding with detect() entirely dead.
    agrees = bool(detect(case.payload.text, case.entry_point)) == expected

    return Result(case.id, contained, expected, got, agrees, detail=why)


def _is_contained(document: str, case: Case, nonce: str) -> tuple[bool, str]:
    """Four conditions, all required. Anchored on the exact markers, not a regex sweep.

    The earlier version compared a 40-character prefix against a block-stripped
    document. Two ways that passed while containment was broken: the prefix never
    reached the second half of a multi-line payload, and the stripping regex — keyed to
    a backreferenced nonce — absorbed a forged close marker into the legitimate span.
    Removing escaping from ``contain()`` was invisible to it.
    """
    open_marker = f"<<<{TOKEN}:{case.entry_point}:{nonce}"
    close_marker = f"{TOKEN}:{case.entry_point}:{nonce}>>>"

    if document.count(open_marker) != 1 or document.count(close_marker) != 1:
        return False, f"expected exactly one marker pair, got {document.count(open_marker)}"

    start = document.index(open_marker) + len(open_marker)
    end = document.index(close_marker)
    if end < start:
        return False, "close marker precedes open marker"
    body = document[start:end].strip("\n")
    outside = document[: document.index(open_marker)] + document[end + len(close_marker) :]

    # 1. The escaping must be lossless: the body is exactly this payload, escaped.
    if unescape(body) != case.payload.text:
        return False, "block body does not unescape to the original payload"
    # 2. No unescaped token may survive inside the body — that is a forged boundary.
    if TOKEN in body:
        return False, "unescaped token inside the block body"
    # 3. The whole payload, not a prefix, must be absent outside the block.
    if case.payload.text in outside:
        return False, "payload appears outside the block"
    # 4. No line of a multi-line payload may appear outside either.
    for line in (ln for ln in case.payload.text.split("\n") if len(ln.strip()) > 12):
        if line in outside:
            return False, f"payload line leaked outside the block: {line[:40]!r}"
    return True, ""


def run_all(*, enabled: bool = True) -> list[Result]:
    return [run_case(case, enabled=enabled) for case in matrix()]


def main() -> int:
    results = run_all()
    for r in results:
        mark = "PASS" if r.passed else "FAIL"
        note = " (finding asserted)" if r.expected_finding else ""
        drift = "" if r.detector_agrees else " DETECTOR DISAGREES WITH THE FIXTURE"
        print(f"{mark}  {r.case_id:<38}contained={r.contained}{note}{drift} {r.detail}")

    detected = [r.case_id for r in results if r.expected_finding]
    undetected = [r.case_id for r in results if not r.expected_finding]
    failed = [r.case_id for r in results if not r.passed]

    print(f"\n  {len(results)} controls, {len(failed)} failure(s)")
    print(f"  detected by the deterministic layer : {len(detected)}")
    print(f"  left to the model layer (#117)      : {len(undetected)}")
    for case_id in undetected:
        print(f"      {case_id}")

    if len(results) != 35:
        print(f"\nFAIL expected 35 controls, ran {len(results)}")
        return 1
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
