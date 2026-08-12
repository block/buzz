"""The attack matrix and the benign corpus.

Payloads are stored bound to no entry point, and crossed with every surface here. That
is deliberate: #120's third done-criterion is that the *same* payload is exercised
through the diff and through a comment, so a corpus that pre-assigned each attack to one
surface would satisfy the letter of "we tested three entry points" while proving nothing
about whether the boundary generalises.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path

from contain import ENTRY_POINTS

FIXTURES = Path(__file__).parent / "fixtures"
PAYLOADS = FIXTURES / "payloads.json"
BENIGN = FIXTURES / "benign.json"


@dataclass(frozen=True)
class Payload:
    id: str
    intent: str
    text: str


@dataclass(frozen=True)
class Case:
    """One cell of the matrix: a payload delivered through one surface."""

    payload: Payload
    entry_point: str

    @property
    def id(self) -> str:
        return f"{self.payload.id}@{self.entry_point}"


def load_payloads(path: Path = PAYLOADS) -> list[Payload]:
    raw = json.loads(path.read_text(encoding="utf-8"))
    return [Payload(p["id"], p["intent"], p["text"]) for p in raw["payloads"]]


def matrix(path: Path = PAYLOADS) -> list[Case]:
    """Every payload through every entry point. 5 x 7 = 35 cases."""
    return [Case(p, ep) for p in load_payloads(path) for ep in ENTRY_POINTS]


def load_benign(path: Path = BENIGN) -> tuple[list[dict], str, str]:
    """Returns (benign records, out-of-scope held-out, in-scope held-out).

    Both held-out payloads live here rather than in payloads.json, so a detector written
    against the attack corpus cannot have been tuned to either. They test opposite
    directions: the in-scope one carries a tell the detector claims to cover and must be
    caught, proving generalisation; the out-of-scope one is a semantic paraphrase and
    must be missed, pinning the documented recall gap so it cannot drift unnoticed.
    """
    raw = json.loads(path.read_text(encoding="utf-8"))
    return raw["records"], raw["held_out"]["text"], raw["held_out_in_scope"]["text"]
