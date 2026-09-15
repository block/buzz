#!/usr/bin/env python3
"""Finite-state NIP-BA retry model; exhaustive reachable-state search.

No runtime dependency. This is a specification model, not a broker emulator.
See NOTE.md for the abstraction, properties, and explicit non-claims.
"""
from collections import deque
from dataclasses import dataclass, replace
import json


@dataclass(frozen=True)
class Record:
    digest: int = -1
    phase: str = "absent"
    dispatches: int = 0
    effects: int = 0
    # Ghost evidence: final verdict before optional result erasure.
    final: str = ""


@dataclass(frozen=True)
class State:
    records: tuple
    # Current auth/release authorization for each context; revocation is monotone.
    allowed: tuple
    # Whether a caller still lacks conclusive evidence of its original attempt.
    uncertain: tuple


CONTEXTS = ((0, 0), (0, 1), (1, 0))  # community, principal


def slot(context, mutant):
    if mutant == "omit_community":
        return CONTEXTS.index((0, CONTEXTS[context][1]))
    if mutant == "omit_principal":
        return CONTEXTS.index((CONTEXTS[context][0], 0))
    return context


def successors(state, mutant=""):
    """(label, next state, observation) transitions, all choices enumerated.

    Observations: (caller context, owner slot, verdict, releases stored data,
    newly introduced effects). Attempt IDs are fixed to one shared ID; digests
    0 and 1 represent unequal complete request bytes.
    """
    def changed(i, record, *, uncertain=None):
        records = list(state.records)
        records[i] = record
        return replace(state, records=tuple(records), uncertain=(
            state.uncertain if uncertain is None else uncertain))

    for c in range(len(CONTEXTS)):
        if state.allowed[c]:
            allowed = list(state.allowed)
            allowed[c] = False
            yield f"revoke({c})", replace(state, allowed=tuple(allowed)), None
        i = slot(c, mutant)
        r = state.records[i]
        for digest in (0, 1):
            prefix = f"submit(context={c},digest={digest})"
            if not state.allowed[c] and mutant != "replay_after_revoke":
                uncertain = state.uncertain
                if mutant == "refusal_clears_uncertainty":
                    uncertain = tuple(False if j == c else u for j, u in enumerate(uncertain))
                yield prefix + ":unauthorized", replace(state, uncertain=uncertain), (c, i, "refusal", False, 0)
            elif r.phase == "absent":
                if state.allowed[c]:
                    yield prefix + ":claim", changed(i, Record(digest, "ready")), None
            elif r.digest != digest:
                if mutant == "ignore_digest":
                    yield prefix + ":wrong-replay", state, (c, i, "wrong_digest", True, 0)
                else:
                    yield prefix + ":conflict", state, (c, i, "refusal", False, 0)
            elif r.phase in ("ready", "running"):
                uncertain = tuple(True if j == c else u for j, u in enumerate(state.uncertain))
                if mutant == "duplicate_dispatch" and r.phase == "running":
                    yield prefix + ":redispatch", changed(i, replace(r, dispatches=r.dispatches + 1)), None
                else:
                    yield prefix + ":wait-timeout", replace(state, uncertain=uncertain), (c, i, "unknown", False, 0)
            else:
                verdict = r.phase if r.phase in ("succeeded", "failed") else "unknown"
                uncertain = tuple((verdict == "unknown") if j == c else u for j, u in enumerate(state.uncertain))
                yield prefix + ":replay", replace(state, uncertain=uncertain), (c, i, verdict, True, 0)

    for i, r in enumerate(state.records):
        if r.phase == "ready":
            if state.allowed[i]:
                yield f"dispatch({i})", changed(i, replace(r, phase="running", dispatches=r.dispatches + 1)), None
            else:
                yield f"deny-before-dispatch({i})", changed(i, replace(r, phase="failed", final="failed")), None
        if r.phase == "running":
            if r.effects == 0:
                # Revocation after authorized dispatch does not recall work already begun.
                yield f"effect({i})", changed(i, replace(r, effects=1)), None
                yield f"finish-no-effects({i})", changed(i, replace(r, phase="failed", final="failed")), None
            else:
                yield f"finish-success({i})", changed(i, replace(r, phase="succeeded", final="succeeded")), None
            yield f"crash({i})", changed(i, replace(r, phase="crashed")), None
        if r.phase == "crashed":
            if mutant == "restart_executor":
                yield f"unsafe-restart({i})", changed(i, replace(r, phase="ready")), None
            if mutant == "false_failure":
                yield f"unsafe-failure({i})", changed(i, replace(r, phase="failed", final="failed")), None
            yield f"persist-unknown({i})", changed(i, replace(r, phase="unknown")), None
        if r.phase in ("crashed", "unknown"):
            # Abstract external reconciliation evidence: it can reveal reality,
            # not create effects. In a real host it may never become available.
            verdict = "succeeded" if r.effects else "failed"
            yield f"reconcile({i},{verdict})", changed(i, replace(r, phase=verdict, final=verdict)), None
        if r.phase in ("succeeded", "failed", "unknown"):
            yield f"erase-result({i})", changed(i, replace(r, phase="tombstone")), None
        if r.phase == "tombstone" and mutant == "evict_protection":
            # Preserve ghost counters so clearing the journal cannot hide a repeat.
            yield f"unsafe-evict({i})", changed(i, replace(r, phase="ready")), None


def violation(before, after, observation):
    for r in after.records:
        if r.dispatches > 1:
            return "at_most_once_dispatch"
        if r.final == "failed" and r.effects:
            return "no_false_failure"
        if r.final == "succeeded" and not r.effects:
            return "success_has_effect_evidence"
    if observation:
        c, owner, verdict, released, effects = observation
        if released and owner != c:
            return "context_isolation"
        if released and not before.allowed[c]:
            return "no_release_after_revocation"
        if verdict == "wrong_digest":
            return "byte_identity"
        if effects:
            return "replay_has_no_effects"
        if verdict == "refusal" and before.uncertain[c] and not after.uncertain[c]:
            return "refusal_preserves_prior_uncertainty"
    return None


def check(mutant="", contexts=1):
    # Smaller runs isolate each mutation; the baseline is also run with all
    # contexts. Unused contexts start revoked, avoiding irrelevant interleavings.
    initial = State(tuple(Record() for _ in CONTEXTS),
                    tuple(i < contexts for i in range(len(CONTEXTS))),
                    (False,) * len(CONTEXTS))
    queue = deque([initial])
    parents = {initial: None}
    edges = 0
    while queue:
        state = queue.popleft()
        for label, nxt, observation in successors(state, mutant):
            edges += 1
            broken = violation(state, nxt, observation)
            if broken:
                trace = [label]
                while parents[state]:
                    state, step = parents[state]
                    trace.append(step)
                return {"mutant": mutant or "baseline", "property": broken,
                        "states": len(parents), "edges": edges,
                        "counterexample": list(reversed(trace))}
            if nxt not in parents:
                parents[nxt] = (state, label)
                queue.append(nxt)
    return {"mutant": mutant or "baseline", "states": len(parents),
            "edges": edges, "counterexample": None}


def main():
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--contexts", type=int, choices=(1, 2, 3), default=3)
    args = parser.parse_args()
    baseline = check(contexts=args.contexts)
    print(json.dumps(baseline, sort_keys=True))
    if baseline["counterexample"]:
        raise SystemExit("baseline invariant failure")
    mutations = {
        "omit_community": (3, "context_isolation"),
        "omit_principal": (2, "context_isolation"),
        "replay_after_revoke": (1, "no_release_after_revocation"),
        "ignore_digest": (1, "byte_identity"),
        "duplicate_dispatch": (1, "at_most_once_dispatch"),
        "restart_executor": (1, "at_most_once_dispatch"),
        "false_failure": (1, "no_false_failure"),
        "evict_protection": (1, "at_most_once_dispatch"),
        "refusal_clears_uncertainty": (1, "refusal_preserves_prior_uncertainty"),
    }
    for mutant, (contexts, expected) in mutations.items():
        result = check(mutant, contexts)
        print(json.dumps(result, sort_keys=True))
        if not result["counterexample"] or result["property"] != expected:
            raise SystemExit(f"mutation did not falsify expected property: {mutant}")


if __name__ == "__main__":
    main()
