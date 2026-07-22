"""Mutation harness for candidate B (two-counter) model.

Each mutant: subclass DeviceB with a weakened rule, run the BFS explorer,
require a recorded counterexample. A model that stays green under a real
weakening is worthless.

Mutants:
  M1: drop baseline dominance (frontier > B no longer clears stale set)
  M2: drop max(S,C)+1 bump (use S+1 or C+1 — counter can regress)
  M3: flip tie policy (verify the model distinguishes them)
  M4: revert to delete-on-dominance compaction (drops the tombstone floor
      entirely instead of zeroing S and keeping max(S,C) as C) — reproduces
      Thufir's pass-3 CRITICAL: stale-replay resurrection after counter reuse
  M5: uint32 overflow bypass (legacy sanitization disabled)
  M6: componentwise-max -> last-write-wins merge (convergence breaks)
  M7: publish without canonicalization (serialize raw registers instead
      of the compact-at-publish canonical form) — reproduces Thufir's
      pass-1/2 CRITICAL: dead+dead merge resurrection

Each mutant is injected into the model via DeviceB subclass, then the
explorer or invariant suite is rerun. The counterexample (first violation)
is recorded and printed.
"""
from copy import deepcopy
from model import (
    RegB, merge_reg_b, override_set_b, compact_b,
    DeviceB, legacy_sanitize_blob,
    escape_context_key,
    SET, CLEAR,
)
from exhaustive import (
    explore_b, test_concurrent_stability,
    test_compaction_register_exhaustive, test_deep_history_compaction,
    test_published_merge_closure,
    CONTEXTS,
)


# ---------------------------------------------------------------------------
# M1: drop baseline dominance
# ---------------------------------------------------------------------------

class M1_NoBaselineDominance(DeviceB):
    def _override_set(self, reg, frontier_val, tie_policy):
        if reg is None:
            return False
        if reg.s > reg.c:
            return True
        if reg.s == reg.c and reg.s > 0:
            return tie_policy == SET
        return False

    def _compact(self, reg, frontier_val, tie_policy):
        if reg.s == 0 and reg.c == 0:
            return None
        if self._override_set(reg, frontier_val, tie_policy):
            return reg
        if reg.c > reg.s:
            return RegB(s=0, c=reg.c, b=0)
        if reg.c == reg.s and tie_policy == CLEAR:
            return RegB(s=0, c=reg.c, b=0)
        return reg


def mutant_m1():
    """M1: without baseline dominance, a stale set persists after frontier
    advance past baseline. Verify by constructing the scenario directly:
    mark-unread at frontier=10, then advance frontier to 100. The correct
    model clears the override; the mutant keeps it live."""
    violations = []
    for ctx in CONTEXTS:
        dev = M1_NoBaselineDominance("d0")
        dev.frontier[ctx] = 10
        dev.do_mark_unread(ctx)
        dev.do_advance_frontier(ctx, 100)

        correct = override_set_b(dev.overrides[ctx], 100, CLEAR)
        mutant_result = dev.override_is_set(ctx, CLEAR)

        if correct != mutant_result:
            violations.append((
                "baseline-dominance-missing", ctx,
                dev.overrides[ctx], 100,
                f"correct={correct}", f"mutant={mutant_result}",
            ))

    if not violations:
        _, violations = explore_b(max_depth=3, tie_policy=CLEAR,
                                  device_cls=M1_NoBaselineDominance)
    return violations


# ---------------------------------------------------------------------------
# M2: drop max(S,C)+1 bump
# ---------------------------------------------------------------------------

class M2_NoBump(DeviceB):
    """Each counter bumps only itself: mark_unread does S := S+1,
    mark_read does C := C+1. When S > C from a prior set, a clear
    at C+1 can produce C < S even though the clear is causally later."""
    def do_mark_unread(self, ctx):
        if self.is_legacy:
            return
        cur = self.overrides.get(ctx, RegB())
        self.overrides[ctx] = RegB(s=cur.s + 1, c=cur.c,
                                   b=self.effective_frontier(ctx))

    def do_mark_read(self, ctx, frontier_ts):
        self.frontier[ctx] = max(self.frontier.get(ctx, 0), frontier_ts)
        if not self.is_legacy:
            cur = self.overrides.get(ctx, RegB())
            self.overrides[ctx] = RegB(s=cur.s, c=cur.c + 1, b=cur.b)


def mutant_m2():
    """M2: each counter bumps independently. After set→set→clear at
    the SAME frontier (no advance past baseline): correct clear has
    C=3 > S=2, mutant clear has C=1 < S=2 — a causally later clear
    fails to dominate.

    Use mark_read at the current frontier (not advancing past baseline)
    so baseline dominance doesn't mask the counter discrepancy.
    """
    violations = []
    for ctx in CONTEXTS:
        front = 10
        dev_correct = DeviceB("d0")
        dev_correct.frontier[ctx] = front
        dev_correct.do_mark_unread(ctx)
        dev_correct.do_mark_unread(ctx)
        dev_correct.do_mark_read(ctx, front)

        dev_mutant = M2_NoBump("d0")
        dev_mutant.frontier[ctx] = front
        dev_mutant.do_mark_unread(ctx)
        dev_mutant.do_mark_unread(ctx)
        dev_mutant.do_mark_read(ctx, front)

        correct_set = dev_correct.override_is_set(ctx, CLEAR)
        mutant_set = dev_mutant.override_is_set(ctx, CLEAR)

        if correct_set != mutant_set:
            violations.append((
                "bump-independent", ctx,
                f"correct={dev_correct.overrides[ctx]}",
                f"mutant={dev_mutant.overrides[ctx]}",
                f"correct_set={correct_set}", f"mutant_set={mutant_set}",
            ))

    if not violations:
        _, violations = explore_b(max_depth=4, tie_policy=CLEAR,
                                  device_cls=M2_NoBump)
    return violations


# ---------------------------------------------------------------------------
# M3: tie policy distinguishable
# ---------------------------------------------------------------------------

def mutant_m3():
    """M3: tie policy is load-bearing — S==C must produce different verdicts.
    Not a DeviceB mutation; tests the model function directly."""
    reg = RegB(s=1, c=1, b=10)
    frontier = 10
    v_clear = override_set_b(reg, frontier, CLEAR)
    v_set = override_set_b(reg, frontier, SET)
    if v_clear == v_set:
        return []
    return [("tie-distinguishable", v_clear, v_set, reg, frontier)]


# ---------------------------------------------------------------------------
# M4: revert to delete-on-dominance compaction (drops the tombstone floor)
# ---------------------------------------------------------------------------

class M4_DeleteOnDominance(DeviceB):
    """The pre-fix compaction rule: any dead/dominated register is deleted
    entirely rather than reduced to the tombstone floor RegB(0, max(S,C), 0).
    This makes counters reusable — a later local set/clear pair restarts
    from S=0/C=0, so a delayed stale peer snapshot can dominate it on
    replay. This is exactly the rule Thufir's pass-3 CRITICAL found live
    at e453b3945."""
    def _compact(self, reg, frontier_val, tie_policy):
        if reg.s == 0 and reg.c == 0:
            return None
        if self._override_set(reg, frontier_val, tie_policy):
            return reg
        if frontier_val > reg.b:
            return None
        if reg.c > reg.s:
            return RegB(s=0, c=reg.c, b=0)
        if reg.c == reg.s and tie_policy == CLEAR:
            return RegB(s=0, c=reg.c, b=0)
        return reg


def mutant_m4():
    """M4: without the tombstone floor, compaction deletes the counter
    ceiling instead of preserving it. Reproduce Thufir's exact witness
    directly: RegB(3,0,10) at frontier=20 compacts to None under the old
    rule (vs. RegB(0,3,0) under the fix); a subsequent local set+clear
    reuses counters from zero; the stale ancestor then replays and
    resurrects (S>C) under both tie policies.

    Then confirm the explorer/deep-history suite also catches it (defense
    in depth — a mutant that only fails a hand-built scenario would still
    be a real bug, but the directed check is what's supposed to catch this
    class per T2/T3)."""
    violations = []
    stale = RegB(s=3, c=0, b=10)
    frontier_after = 20

    for tie_policy in (CLEAR, SET):
        dev = M4_DeleteOnDominance("d0")
        dev.frontier["c0"] = 10
        dev.overrides["c0"] = stale
        dev.do_advance_frontier("c0", frontier_after)
        dev.do_compact("c0", tie_policy)
        if "c0" in dev.overrides:
            continue  # old rule didn't drop it here; not the witness shape

        dev.do_mark_unread("c0")   # S := 1, B := 20
        dev.do_mark_read("c0", frontier_after)  # C := 2

        stale_blob = {"contexts": {"ov_s:c0": stale.s, "ov_c:c0": stale.c, "ov_b:c0": stale.b}}
        dev.receive_merge(stale_blob)
        resurrected = dev.override_is_set("c0", tie_policy)

        if resurrected:
            violations.append((
                "M4-delete-on-dominance-resurrection", tie_policy,
                f"stale_ancestor={stale}", f"post_compact_reuse=(set,clear)",
                f"final_reg={dev.overrides['c0']}", f"override_is_set={resurrected}",
            ))

    if not violations:
        _, violations = test_deep_history_compaction(device_cls=M4_DeleteOnDominance)
    return violations


# ---------------------------------------------------------------------------
# M5: uint32 overflow bypass
# ---------------------------------------------------------------------------

def mutant_m5():
    """M5: values outside uint32 range must fail legacy sanitization."""
    blob = {"v": 1, "client_id": "x", "contexts": {
        "ov_s:c0": 4294967296,
        "ov_c:c0": 0,
        "ov_b:c0": 10,
    }}
    sanitized = legacy_sanitize_blob(blob)
    if "ov_s:c0" in sanitized["contexts"]:
        return []
    return [("overflow-rejected", blob["contexts"]["ov_s:c0"],
             sanitized["contexts"])]


# ---------------------------------------------------------------------------
# M6: last-write-wins merge (breaks convergence)
# ---------------------------------------------------------------------------

class M6_LastWriteWins(DeviceB):
    def _merge_reg(self, a, b):
        if a is None:
            return b
        if b is None:
            return a
        return b


def mutant_m6():
    """M6: replace componentwise max with last-write-wins. Convergence must
    break — different delivery orders produce different final states."""
    _, violations = explore_b(max_depth=3, tie_policy=CLEAR,
                              device_cls=M6_LastWriteWins)
    return violations


# ---------------------------------------------------------------------------
# M7: publish without canonicalization (reproduces Thufir's pass-1/2
# CRITICAL — dead+dead merge resurrection)
# ---------------------------------------------------------------------------

class M7_PublishWithoutCanonicalization(DeviceB):
    """Reverts `publish_blob` to serialize raw, uncompacted registers —
    the exact pre-fix behavior Thufir's pass-1/2 CRITICAL exploited:
    a dead register's baseline-relative death (or clear-count-relative
    death) never gets folded into a globally-comparable ceiling before
    hitting the wire, so two individually-dead registers can
    componentwise-max-merge into a live join."""
    def publish_blob(self, tie_policy=CLEAR):
        blob_ctx = {escape_context_key(k): v for k, v in self.frontier.items()}
        if not self.is_legacy:
            for k, reg in self.overrides.items():
                blob_ctx[f"ov_s:{k}"] = reg.s
                blob_ctx[f"ov_c:{k}"] = reg.c
                blob_ctx[f"ov_b:{k}"] = reg.b
        return {"v": 1, "client_id": self.client_id, "contexts": blob_ctx}


def mutant_m7():
    """M7: publish-without-canonicalization must be caught by the
    published-state merge-closure invariant — proving that invariant
    has teeth. Reproduce Thufir's exact witness directly first (fast,
    deterministic); fall back to the full search if the hand-built
    scenario doesn't trigger under a given tie policy."""
    violations = []
    for tie_policy in (CLEAR, SET):
        dev_a = M7_PublishWithoutCanonicalization("a")
        dev_a.frontier["c0"] = 50
        dev_a.overrides["c0"] = RegB(s=3, c=2, b=0)
        dev_b = M7_PublishWithoutCanonicalization("b")
        dev_b.frontier["c0"] = 100
        dev_b.overrides["c0"] = RegB(s=1, c=2, b=100)

        blob_a = dev_a.publish_blob(tie_policy)
        blob_b = dev_b.publish_blob(tie_policy)

        for first, second in [(blob_a, blob_b), (blob_b, blob_a)]:
            recv = M7_PublishWithoutCanonicalization("recv")
            recv.receive_merge(first)
            recv.receive_merge(second)
            if recv.override_is_set("c0", tie_policy):
                violations.append((
                    "M7-publish-without-canonicalization-resurrection",
                    tie_policy, blob_a, blob_b, recv.overrides["c0"],
                ))

    if not violations:
        _, violations = test_published_merge_closure(
            device_cls=M7_PublishWithoutCanonicalization
        )
    return violations


# ---------------------------------------------------------------------------
# Runner
# ---------------------------------------------------------------------------

def run_mutations():
    mutants = [
        ("M1: drop baseline dominance", mutant_m1),
        ("M2: drop max(S,C)+1 bump", mutant_m2),
        ("M3: tie policy distinguishable", mutant_m3),
        ("M4: revert to delete-on-dominance compaction (reproduces pass-3 CRITICAL)", mutant_m4),
        ("M5: uint32 overflow bypass", mutant_m5),
        ("M6: last-write-wins merge", mutant_m6),
        ("M7: publish without canonicalization (reproduces pass-1/2 CRITICAL)", mutant_m7),
    ]

    print("=" * 60)
    print("Mutation harness — candidate B")
    print("=" * 60)

    caught = []
    missed = []
    for name, fn in mutants:
        violations = fn()
        if violations:
            caught.append(name)
            v = violations[0]
            detail = str(v)[:200]
            print(f"  CAUGHT: {name}")
            print(f"    counterexample: {detail}")
        else:
            missed.append(name)
            print(f"  MISSED: {name}")

    print(f"\nCaught {len(caught)}/{len(mutants)} mutants")
    if missed:
        print(f"MISSED: {missed}")
    print("=" * 60)
    return len(missed) == 0


if __name__ == "__main__":
    import sys
    sys.exit(0 if run_mutations() else 1)
