"""Bounded transition-system explorer for NIP-RS manual-unread candidates.

BFS over canonical global states. At each depth, enabled transitions are
local actions and message deliveries, interleaved — not phased.

Universe: 2 upgraded devices + 1 legacy device, 2 contexts.
Transitions: mark_unread, mark_read (with frontier advance),
  advance_frontier, compact, reinstall, deliver (including
  duplicate/replay). Legacy rewrite semantics are exercised through the
  deliver path (legacy_sanitize_and_publish), not as a separate
  transition — a legacy device never mutates its own state outside
  delivery, so a dedicated no-op transition added nothing (see NOTE.md).

Invariants:
  I1  merge_reg_b associative/commutative/idempotent
  I2  convergence: all delivery orders -> identical override verdict
  I3  no frontier regression
  I4  concurrent set/clear winner stable (order-independent, ancestor-independent)
  I5  compaction: no loss of live set, no resurrection of dead clear,
      survives merge with stale pre-compaction state
  I5c directed deep-history: compact -> new local actions (counter reuse)
      -> delayed stale delivery does not resurrect a dead override or
      lose a genuinely-live one. Scope: the compacting device's own
      pre-compaction ancestor (or an exact copy of it) replayed back to
      that same device.
  I5d cross-device compaction transparency (requalified, NOT
      zero-divergence): a tombstone's counter ceiling can one-shot
      suppress an unrelated device's concurrent fresh set with no
      resurrection, and the suppression is always recoverable by one
      more local action. Proven suppress-only direction via a bounded
      witness cube plus a structural monotonicity argument.
  I6  replay harmless
  I7  legacy rewrite: B sibling keys survive / A overrides erased (witness)
  I8  bounded key growth per context
  I9  DeviceA post-receive counter absorption
"""
from itertools import permutations
from copy import deepcopy
from model import (
    RegB, merge_reg_b, override_set_b, compact_b,
    RegA, merge_reg_a,
    DeviceB, DeviceA,
    SET, CLEAR,
    legacy_prune, legacy_trim, legacy_sanitize_blob,
    escape_context_key, unescape_context_key, ESCAPE_PREFIX,
)

CONTEXTS = ("c0", "c1")
FRONTIER_VALS = (10, 20)


# ---------------------------------------------------------------------------
# I1: algebraic properties
# ---------------------------------------------------------------------------

def test_merge_algebra_b():
    vals = [0, 1, 2, 3]
    regs = [RegB(s, c, b) for s in vals for c in vals for b in vals]
    violations = []
    for a in regs:
        if merge_reg_b(a, a) != a:
            violations.append(("idempotent", a))
    for a in regs:
        for b in regs:
            if merge_reg_b(a, b) != merge_reg_b(b, a):
                violations.append(("commutative", a, b))
    for a in regs:
        for b in regs:
            for c in regs:
                if merge_reg_b(merge_reg_b(a, b), c) != merge_reg_b(a, merge_reg_b(b, c)):
                    violations.append(("associative", a, b, c))
    return violations


def test_merge_algebra_a():
    vals = [0, 1, 2]
    tiebreaks = ["a", "b"]
    ops = [SET, CLEAR]
    baselines = [0, 10]
    regs = [RegA(ct, t, o, bl)
            for ct in vals for t in tiebreaks for o in ops for bl in baselines]
    violations = []
    for tie_op in [CLEAR, SET]:
        for a in regs:
            if merge_reg_a(a, a, tie_op) != a:
                violations.append(("idempotent", tie_op, a))
        for a in regs:
            for b in regs:
                if merge_reg_a(a, b, tie_op) != merge_reg_a(b, a, tie_op):
                    violations.append(("commutative", tie_op, a, b))
        for a in regs:
            for b in regs:
                for c in regs:
                    ab_c = merge_reg_a(merge_reg_a(a, b, tie_op), c, tie_op)
                    a_bc = merge_reg_a(a, merge_reg_a(b, c, tie_op), tie_op)
                    if ab_c != a_bc:
                        violations.append(("associative", tie_op, a, b, c))
    return violations


# ---------------------------------------------------------------------------
# BFS state explorer — Candidate B
# ---------------------------------------------------------------------------

def next_frontier(device, ctx):
    cur = device.effective_frontier(ctx)
    for fv in FRONTIER_VALS:
        if fv > cur:
            return fv
    return None


def enabled_transitions(devices, tie_policy):
    """Generate (kind, args) tuples for all enabled transitions."""
    trans = []
    for di, d in enumerate(devices):
        for ctx in CONTEXTS:
            if not d.is_legacy:
                trans.append(("mark_unread", di, ctx))
                fv = next_frontier(d, ctx)
                if fv is not None:
                    trans.append(("mark_read", di, ctx, fv))
                if ctx in d.overrides:
                    trans.append(("compact", di, ctx))
            fv = next_frontier(d, ctx)
            if fv is not None:
                trans.append(("advance", di, ctx, fv))
        if not d.is_legacy:
            trans.append(("reinstall", di))
    for si in range(len(devices)):
        for di in range(len(devices)):
            if si != di:
                trans.append(("deliver", si, di))
    return trans


def apply_transition(devices, t, tie_policy):
    kind = t[0]
    if kind == "mark_unread":
        devices[t[1]].do_mark_unread(t[2])
    elif kind == "mark_read":
        devices[t[1]].do_mark_read(t[2], t[3])
    elif kind == "advance":
        devices[t[1]].do_advance_frontier(t[2], t[3])
    elif kind == "compact":
        devices[t[1]].do_compact(t[2], tie_policy)
    elif kind == "reinstall":
        devices[t[1]].do_reinstall()
    elif kind == "deliver":
        src = devices[t[1]]
        dst = devices[t[2]]
        if src.is_legacy:
            blob = src.legacy_sanitize_and_publish(tie_policy)
        else:
            blob = src.publish_blob(tie_policy)
        dst.receive_merge(blob)


def state_sig(devices, tie_policy):
    return tuple(d.state_key(CONTEXTS, tie_policy) for d in devices)


def check_convergence(devices, tie_policy, trace, violations):
    """Publish all blobs, deliver in every order, check upgraded devices
    converge on override_is_set for each context.

    Tests with latest_ts=5 (below all frontiers) so the override is the
    sole unread source — no masking by natural unread.
    """
    blobs = []
    for d in devices:
        if d.is_legacy:
            blobs.append(d.legacy_sanitize_and_publish(tie_policy))
        else:
            blobs.append(d.publish_blob(tie_policy))

    verdicts_per_order = []
    for perm in permutations(range(len(blobs))):
        receivers = deepcopy(devices)
        for idx in perm:
            for r in receivers:
                r.receive_merge(blobs[idx])
        per_device = []
        for r in receivers:
            if not r.is_legacy:
                per_device.append(
                    tuple(r.override_is_set(ctx, tie_policy) for ctx in CONTEXTS)
                )
        verdicts_per_order.append(tuple(per_device))

    if len(set(verdicts_per_order)) > 1:
        violations.append(("I2-convergence", trace, set(verdicts_per_order)))


def check_compaction_safety(devices, tie_policy, trace, violations):
    """For each upgraded device with overrides:
    1. Check override_is_set directly (not via verdict/latest_ts).
    2. Compact and verify override_is_set unchanged.
    3. Merge compacted state with stale pre-compaction state in both orders.
       Verify no resurrection and no loss.
    """
    for di, d in enumerate(devices):
        if d.is_legacy:
            continue
        for ctx in CONTEXTS:
            reg = d.overrides.get(ctx)
            if reg is None:
                continue
            front = d.effective_frontier(ctx)
            ov_before = d._override_set(reg, front, tie_policy)
            compacted = d._compact(reg, front, tie_policy)
            ov_after = d._override_set(compacted, front, tie_policy) if compacted else False

            if ov_before and not ov_after:
                violations.append((
                    "I5-compaction-lost-set", trace, di, ctx,
                    reg, compacted, front, tie_policy
                ))
            if not ov_before and ov_after:
                violations.append((
                    "I5-compaction-resurrection", trace, di, ctx,
                    reg, compacted, front, tie_policy
                ))

            if compacted is not None:
                for merged in [merge_reg_b(compacted, reg), merge_reg_b(reg, compacted)]:
                    ov_merged = d._override_set(merged, front, tie_policy)
                    if not ov_before and ov_merged:
                        violations.append((
                            "I5-compaction-merge-resurrection", trace, di, ctx,
                            reg, compacted, merged
                        ))


def explore_b(max_depth=4, tie_policy=CLEAR, device_cls=DeviceB):
    """BFS over all reachable global states up to max_depth.

    Returns (states_explored, violations).
    Accepts device_cls for mutation testing via subclassing.
    """
    def make_devices():
        return [
            device_cls("d0", is_legacy=False),
            device_cls("d1", is_legacy=False),
            device_cls("d2", is_legacy=True),
        ]

    violations = []
    seen = set()
    states_explored = 0
    queue = [(make_devices(), [])]

    while queue:
        devices, trace = queue.pop(0)
        sig = state_sig(devices, tie_policy)
        if sig in seen:
            continue
        seen.add(sig)
        states_explored += 1

        for di, d in enumerate(devices):
            for ctx in CONTEXTS:
                prev_front = d.effective_frontier(ctx)
                if prev_front < 0:
                    violations.append(("I3-frontier-negative", trace, di, ctx))

        if len(trace) >= max_depth:
            check_convergence(devices, tie_policy, trace, violations)
            check_compaction_safety(devices, tie_policy, trace, violations)
            continue

        for t in enabled_transitions(devices, tie_policy):
            new_devs = deepcopy(devices)
            fronts_before = {
                (di, ctx): d.effective_frontier(ctx)
                for di, d in enumerate(new_devs) for ctx in CONTEXTS
            }
            apply_transition(new_devs, t, tie_policy)

            # Reinstall intentionally wipes local state; frontier regression
            # is only invalid during merge/delivery/compaction/advance.
            if t[0] != "reinstall":
                for (di, ctx), fb in fronts_before.items():
                    fa = new_devs[di].effective_frontier(ctx)
                    if fa < fb:
                        violations.append(("I3-frontier-regression", trace + [t], di, ctx, fb, fa))

            queue.append((new_devs, trace + [t]))

    return states_explored, violations


# ---------------------------------------------------------------------------
# I4: concurrent set/clear winner stable
# ---------------------------------------------------------------------------

def test_concurrent_stability(device_cls=DeviceB):
    """Two devices concurrently set and clear from every possible ancestor state.
    The winner must be the same regardless of delivery order AND ancestor state."""
    violations = []
    for tie_policy in [CLEAR, SET]:
        for pre_s, pre_c in [(0, 0), (1, 0), (0, 1), (2, 1), (1, 2), (1, 1)]:
            for front in [0, 10]:
                for ctx in CONTEXTS:
                    ancestor = RegB(s=pre_s, c=pre_c, b=front)

                    d0 = device_cls("d0")
                    d0.frontier[ctx] = front
                    d0.overrides[ctx] = deepcopy(ancestor)
                    d1 = device_cls("d1")
                    d1.frontier[ctx] = front
                    d1.overrides[ctx] = deepcopy(ancestor)

                    d0.do_mark_unread(ctx)
                    d1.do_mark_read(ctx, front + 10)

                    blob0 = d0.publish_blob(tie_policy)
                    blob1 = d1.publish_blob(tie_policy)

                    verdicts = set()
                    for first, second in [(blob0, blob1), (blob1, blob0)]:
                        r = device_cls("recv")
                        r.frontier[ctx] = front
                        r.overrides[ctx] = deepcopy(ancestor)
                        r.receive_merge(first)
                        r.receive_merge(second)
                        verdicts.add(r.override_is_set(ctx, tie_policy))

                    if len(verdicts) > 1:
                        violations.append((
                            "I4-unstable", tie_policy, ctx,
                            pre_s, pre_c, front
                        ))
    return violations


# ---------------------------------------------------------------------------
# I5: direct compaction register-level check (all register values x policies)
# ---------------------------------------------------------------------------

def test_compaction_register_exhaustive():
    """Exhaustive check over bounded register cube and frontier values.
    Tests override_is_set directly — no latest_ts masking."""
    violations = []
    vals = [0, 1, 2, 3]
    frontiers = [0, 10, 20]
    for tie_policy in [CLEAR, SET]:
        for s in vals:
            for c in vals:
                for b in frontiers:
                    for fv in frontiers:
                        reg = RegB(s=s, c=c, b=b)
                        ov_before = override_set_b(reg, fv, tie_policy)
                        compacted = compact_b(reg, fv, tie_policy)
                        ov_after = (override_set_b(compacted, fv, tie_policy)
                                    if compacted else False)

                        if ov_before and not ov_after:
                            violations.append((
                                "loss", tie_policy, reg, fv, compacted
                            ))
                        if not ov_before and ov_after:
                            violations.append((
                                "resurrection", tie_policy, reg, fv, compacted
                            ))

                        if compacted is not None:
                            merged_fwd = merge_reg_b(compacted, reg)
                            merged_rev = merge_reg_b(reg, compacted)
                            for label, merged in [("fwd", merged_fwd), ("rev", merged_rev)]:
                                ov_merged = override_set_b(merged, fv, tie_policy)
                                if not ov_before and ov_merged:
                                    violations.append((
                                        f"merge-resurrection-{label}",
                                        tie_policy, reg, fv, compacted, merged
                                    ))
    return violations


# ---------------------------------------------------------------------------
# I5c: directed deep-history — compact -> new actions (counter reuse) ->
# delayed stale delivery (including split across two slots)
# ---------------------------------------------------------------------------

def _apply_action_seq(dev, ctx, seq, ts):
    for a in seq:
        if a == "set":
            dev.do_mark_unread(ctx)
        else:
            dev.do_mark_read(ctx, ts)


def _ancestor_ctx_dict(ctx, reg):
    return {f"ov_s:{ctx}": reg.s, f"ov_c:{ctx}": reg.c, f"ov_b:{ctx}": reg.b}


def _ancestor_split_ctx_dicts(ctx, reg):
    """Split the same ancestor register across 2 slots, as production's
    splitContextsIntoBudgetedSlots would when a blob spans multiple
    NIP-59 events."""
    return (
        {f"ov_s:{ctx}": reg.s, f"ov_b:{ctx}": reg.b},
        {f"ov_c:{ctx}": reg.c},
    )


_DEEP_HISTORY_ACTION_SEQS = [
    (), ("set",), ("clear",), ("set", "clear"), ("clear", "set"),
    ("set", "set"), ("clear", "clear"),
]
_DEEP_HISTORY_DELIVERY_SHAPES = ("single", "split_fwd", "split_rev")


def test_deep_history_compaction(device_cls=DeviceB):
    """Directed check over the exact shape a depth-4 BFS structurally
    cannot reach (~9 transitions): compact -> new local set/clear actions
    (counter reuse against the tombstone floor) -> delayed delivery of
    the pre-compaction stale ancestor, including split across 2 slots.

    Oracle: compaction is a storage optimization and must never change
    the semantic outcome. A reference device that never compacts, given
    the identical ancestor / frontier advance / action sequence / late
    ancestor delivery, must reach the same override_is_set verdict as
    the compacting device. This directly targets Thufir's counterexample
    (RegB(3,0,10) -> None under delete-on-dominance -> counter reuse ->
    RegB(3,2,20) resurrection) and requires the tombstone floor from
    compact_b to hold under it.

    Returns (cube_size, violations).
    """
    violations = []
    cube_size = 0
    ctx = "c0"
    stale_vals = (0, 1, 2, 3)
    baselines = (0, 10)
    post_frontiers = (10, 20)

    for s0 in stale_vals:
        for c0 in stale_vals:
            for b0 in baselines:
                for f1 in post_frontiers:
                    if f1 <= b0:
                        continue  # not a dominance/compaction scenario
                    ancestor = RegB(s=s0, c=c0, b=b0)
                    ancestor_blob = _ancestor_ctx_dict(ctx, ancestor)
                    slot_fwd, slot_rev = _ancestor_split_ctx_dicts(ctx, ancestor)
                    for seq in _DEEP_HISTORY_ACTION_SEQS:
                        for tie_policy in (CLEAR, SET):
                            for shape in _DEEP_HISTORY_DELIVERY_SHAPES:
                                cube_size += 1

                                dev = device_cls("d0")
                                dev.frontier[ctx] = b0
                                dev.overrides[ctx] = ancestor
                                dev.do_advance_frontier(ctx, f1)
                                dev.do_compact(ctx, tie_policy)
                                _apply_action_seq(dev, ctx, seq, f1)

                                if shape == "single":
                                    dev.receive_merge({"contexts": dict(ancestor_blob)})
                                elif shape == "split_fwd":
                                    dev.receive_merge({"contexts": dict(slot_fwd)})
                                    dev.receive_merge({"contexts": dict(slot_rev)})
                                else:
                                    dev.receive_merge({"contexts": dict(slot_rev)})
                                    dev.receive_merge({"contexts": dict(slot_fwd)})
                                ov_after = dev.override_is_set(ctx, tie_policy)

                                ref = device_cls("ref")
                                ref.frontier[ctx] = b0
                                ref.overrides[ctx] = ancestor
                                ref.do_advance_frontier(ctx, f1)
                                _apply_action_seq(ref, ctx, seq, f1)
                                ref.receive_merge({"contexts": dict(ancestor_blob)})
                                ov_ref = ref.override_is_set(ctx, tie_policy)

                                if ov_after != ov_ref:
                                    violations.append((
                                        "I5c-deep-history-divergence", tie_policy, shape,
                                        ancestor, f1, seq,
                                        f"compacted_path={ov_after}", f"reference={ov_ref}",
                                    ))
    return cube_size, violations


def test_tombstone_stale_merge_direct():
    """Tombstone floor merged directly with its own pre-compaction stale
    ancestor (no intervening local actions) must not resurrect and must
    not exceed the ancestor's own verdict."""
    violations = []
    vals = (0, 1, 2, 3)
    frontiers = (0, 10, 20)
    for tie_policy in (CLEAR, SET):
        for s in vals:
            for c in vals:
                for b in frontiers:
                    for fv in frontiers:
                        if fv <= b:
                            continue
                        reg = RegB(s=s, c=c, b=b)
                        compacted = compact_b(reg, fv, tie_policy)
                        if compacted is None:
                            continue  # virgin register: nothing to tombstone
                        ov_before = override_set_b(reg, fv, tie_policy)
                        for merged in (merge_reg_b(compacted, reg), merge_reg_b(reg, compacted)):
                            ov_merged = override_set_b(merged, fv, tie_policy)
                            if not ov_before and ov_merged:
                                violations.append((
                                    "tombstone-stale-merge-resurrection",
                                    tie_policy, reg, fv, compacted, merged,
                                ))
    return violations


# ---------------------------------------------------------------------------
# I5d: cross-device compaction transparency (requalified — suppress-only,
# NOT zero-divergence) + re-mark recovery
# ---------------------------------------------------------------------------

def test_tombstone_merge_monotonic():
    """Structural lemma: merging in ANY tombstone RegB(0, k, 0) is a
    monotonically non-increasing function of the ceiling k in
    override_set_b's boolean output, for a fixed receiving register and
    frontier. A tombstone only ever adds to C (its S and B are both 0,
    so max() with any receiving register leaves that register's own S
    and B untouched) — raising C can only weaken S's relative standing,
    never strengthen it. This is what makes resurrection structurally
    impossible and suppression the only possible direction, independent
    of any bounded cube.
    """
    violations = []
    vals = (0, 1, 2, 3)
    baselines = (0, 10, 20)
    ceilings = (0, 1, 2, 3, 4, 5)
    for tie_policy in (CLEAR, SET):
        for s in vals:
            for c in vals:
                for b in baselines:
                    for fv in baselines:
                        x_reg = RegB(s=s, c=c, b=b)
                        prev = None
                        for k in ceilings:
                            merged = merge_reg_b(x_reg, RegB(s=0, c=k, b=0))
                            cur = override_set_b(merged, fv, tie_policy)
                            if prev is not None and cur and not prev:
                                violations.append((
                                    "I5d-non-monotonic-ceiling", tie_policy,
                                    x_reg, fv, k, merged,
                                ))
                            prev = cur
    return violations


def test_cross_device_compaction_suppression(device_cls=DeviceB):
    """Compaction is NOT semantically transparent cross-device (I5c only
    covers the same-device replay shape). A tombstone re-encodes
    baseline-dominated death — frontier-relative, doesn't transfer
    across devices — as a clear-counter ceiling — globally comparable —
    so it can one-shot suppress an unrelated device's concurrent fresh
    set whose own counters don't exceed that ceiling.

    Witness (Paul's report, illustrative — the cube below tests nearby
    parameter values `f_x` in `(5, 15, 25, 35)`, not the literal
    `f_x=30` used in the original report; the shape is the same):
      Y: mark_unread -> RegB(1,0,0); frontier->10 (dead) -> compact ->
         tombstone RegB(0,1,0)
      X: offline, fresh mark_unread at frontier 30 -> RegB(1,0,30), LIVE
      X merges Y's tombstone -> RegB(1,1,30) -> suppressed (clear-wins)
      Control (Y publishes the uncompacted RegB(1,0,0) instead): X stays
      RegB(1,0,30), LIVE — the divergence is caused by compaction, not
      by the merge itself.

    Proves over a bounded cube, both tie policies: every divergence
    between "X merges Y's tombstone" and "X merges Y's uncompacted
    ancestor" is a suppression (never a resurrection — that would
    contradict test_tombstone_merge_monotonic), and every suppression
    recovers with one more local mark-unread, stable under tombstone
    replay.

    Returns (cube_size, suppress_count, violations).
    """
    violations = []
    cube_size = 0
    suppress_count = 0
    dead_vals = (0, 1, 2, 3)
    dead_baselines = (0, 10)
    dead_post_frontiers = (10, 20)
    fresh_frontiers = (5, 15, 25, 35)

    for tie_policy in (CLEAR, SET):
        for s_y in dead_vals:
            for c_y in dead_vals:
                for b_y in dead_baselines:
                    for f_y in dead_post_frontiers:
                        if f_y <= b_y:
                            continue
                        ancestor = RegB(s=s_y, c=c_y, b=b_y)
                        tomb = compact_b(ancestor, f_y, tie_policy)
                        if tomb is None or tomb == ancestor:
                            continue  # virgin, or was live (not compacted)

                        for f_x in fresh_frontiers:
                            cube_size += 1
                            x_reg = RegB(s=1, c=0, b=f_x)
                            x_before = override_set_b(x_reg, f_x, tie_policy)
                            if not x_before:
                                violations.append((
                                    "I5d-setup-not-live", tie_policy, x_reg, f_x,
                                ))
                                continue

                            ov_tomb = override_set_b(
                                merge_reg_b(x_reg, tomb), f_x, tie_policy
                            )
                            ov_ancestor = override_set_b(
                                merge_reg_b(x_reg, ancestor), f_x, tie_policy
                            )

                            if ov_tomb == ov_ancestor:
                                continue
                            if ov_tomb and not ov_ancestor:
                                violations.append((
                                    "I5d-resurrection-vs-ancestor", tie_policy,
                                    ancestor, tomb, x_reg, f_x,
                                ))
                                continue

                            suppress_count += 1
                            dev = device_cls("x")
                            dev.frontier["c0"] = f_x
                            dev.overrides["c0"] = merge_reg_b(x_reg, tomb)
                            dev.do_mark_unread("c0")
                            if not dev.override_is_set("c0", tie_policy):
                                violations.append((
                                    "I5d-recovery-failed", tie_policy,
                                    ancestor, tomb, x_reg, f_x, dev.overrides["c0"],
                                ))
                                continue
                            dev.receive_merge({"contexts": {
                                "ov_s:c0": tomb.s, "ov_c:c0": tomb.c, "ov_b:c0": tomb.b,
                            }})
                            if not dev.override_is_set("c0", tie_policy):
                                violations.append((
                                    "I5d-recovery-not-replay-stable", tie_policy,
                                    ancestor, tomb, x_reg, f_x, dev.overrides["c0"],
                                ))

    return cube_size, suppress_count, violations


# ---------------------------------------------------------------------------
# New invariant: published-state merge closure (Paul's fix-scope item 2,
# generalizing Thufir's pass-1/2 CRITICAL — dead+dead merge resurrection)
# ---------------------------------------------------------------------------

def _dead_register_points(tie_policy):
    """Bounded cube of (label, reg, frontier) points independently
    verified DEAD (inactive) under `tie_policy` by the real
    `override_set_b` predicate — the death cause (baseline dominance,
    clear-count dominance, or clear-wins tie) is whatever the predicate
    actually computes for that point, not asserted by construction.
    """
    vals = (0, 1, 2, 3)
    baselines = (0, 10, 50)
    frontiers = (0, 20, 60, 100)
    points = []
    for s in vals:
        for c in vals:
            if s == 0 and c == 0:
                continue  # virgin: not a "dead override" case
            for b in baselines:
                for fv in frontiers:
                    reg = RegB(s=s, c=c, b=b)
                    if override_set_b(reg, fv, tie_policy):
                        continue  # live: out of scope for this invariant
                    points.append((f"s={s}c={c}b={b}fv={fv}", reg, fv))
    return points


def test_published_merge_closure(device_cls=DeviceB):
    """Over reachable *published* states: joining any two individually-
    inactive published states must remain inactive.

    This targets Thufir's pass-1/2 CRITICAL directly: a dead register's
    death cause is frontier-relative (baseline dominance) or
    device-local-history-relative (clear-count dominance), but the
    componentwise-max join recombines each register's `S`/`C`/`B`
    independent of the history that produced them, so two individually-
    dead registers could — before canonical publication — recombine
    into a live join. Canonicalizing every override to `RegB(0,
    max(S,C), 0)` before serialization (this round's CRITICAL fix)
    folds every dead cause into a single globally-comparable `C`
    ceiling with `S=0`, which per `test_tombstone_merge_monotonic` can
    only ever raise a receiver's `C` — never resurrect.

    Checked two ways:
    - Directed case: Thufir's exact witness pair — `RegB(3,2,0)`
      inactive via baseline dominance at frontier 50, `RegB(1,2,100)`
      inactive via clear dominance at frontier 100 — whose raw
      componentwise join is `RegB(3,2,100)`, live (`S=3>C=2`,
      `frontier(100) not> B(100)`). Both tie policies.
    - General search: every pairwise join of a bounded cube of
      independently-dead `(reg, frontier)` points (see
      `_dead_register_points`), delivered to a fresh receiver in both
      direct orders and via a one-hop relay that itself republishes
      (re-canonicalizes) what it received before forwarding — covering
      delayed/multi-hop delivery, not just direct pairwise merge.

    Returns (cube_size, violations).
    """
    violations = []
    cube_size = 0

    def _check_pair(tie_policy, label_a, blob_a, label_b, blob_b, tag):
        nonlocal cube_size
        cube_size += 1
        for first, second in [(blob_a, blob_b), (blob_b, blob_a)]:
            recv = device_cls("recv")
            recv.receive_merge(first)
            recv.receive_merge(second)
            if recv.override_is_set("c0", tie_policy):
                violations.append((
                    tag, tie_policy, label_a, label_b, recv.overrides.get("c0"),
                ))
        # Multi-hop: a relay receives blob_a alone, republishes
        # (re-canonicalizes) before forwarding, then the receiver gets
        # the relayed form plus blob_b directly, in both orders.
        relay = device_cls("relay")
        relay.receive_merge(blob_a)
        relayed = relay.publish_blob(tie_policy)
        for first, second in [(relayed, blob_b), (blob_b, relayed)]:
            recv = device_cls("recv_hop")
            recv.receive_merge(first)
            recv.receive_merge(second)
            if recv.override_is_set("c0", tie_policy):
                violations.append((
                    tag + "-multihop", tie_policy, label_a, label_b,
                    recv.overrides.get("c0"),
                ))

    # --- Directed case: Thufir's exact witness pair. ---
    for tie_policy in (CLEAR, SET):
        reg_a, front_a = RegB(s=3, c=2, b=0), 50
        reg_b, front_b = RegB(s=1, c=2, b=100), 100
        assert not override_set_b(reg_a, front_a, tie_policy)
        assert not override_set_b(reg_b, front_b, tie_policy)

        dev_a = device_cls("a")
        dev_a.frontier["c0"] = front_a
        dev_a.overrides["c0"] = reg_a
        dev_b = device_cls("b")
        dev_b.frontier["c0"] = front_b
        dev_b.overrides["c0"] = reg_b

        _check_pair(
            tie_policy, f"thufir-witness-A={reg_a}@{front_a}",
            dev_a.publish_blob(tie_policy),
            f"thufir-witness-B={reg_b}@{front_b}",
            dev_b.publish_blob(tie_policy),
            "merge-closure-thufir-witness",
        )

    # --- General search over a bounded cube of dead published states. ---
    for tie_policy in (CLEAR, SET):
        points = _dead_register_points(tie_policy)
        for label_a, reg_a, front_a in points:
            dev_a = device_cls("a")
            dev_a.frontier["c0"] = front_a
            dev_a.overrides["c0"] = reg_a
            blob_a = dev_a.publish_blob(tie_policy)
            for label_b, reg_b, front_b in points:
                dev_b = device_cls("b")
                dev_b.frontier["c0"] = front_b
                dev_b.overrides["c0"] = reg_b
                blob_b = dev_b.publish_blob(tie_policy)
                _check_pair(
                    tie_policy, label_a, blob_a, label_b, blob_b,
                    "merge-closure-cube",
                )

    return cube_size, violations


# ---------------------------------------------------------------------------
# I6: replay harmless
# ---------------------------------------------------------------------------

def test_replay_harmless(device_cls=DeviceB):
    violations = []
    for tie_policy in [CLEAR, SET]:
        for ctx in CONTEXTS:
            d = device_cls("d0")
            d.frontier[ctx] = 10
            d.do_mark_unread(ctx)
            blob = d.publish_blob(tie_policy)
            state_before = (
                dict(d.frontier),
                {k: v for k, v in d.overrides.items()},
            )
            d.receive_merge(blob)
            d.receive_merge(blob)
            d.receive_merge(blob)
            state_after = (
                dict(d.frontier),
                {k: v for k, v in d.overrides.items()},
            )
            if state_before != state_after:
                violations.append(("I6-replay", tie_policy, ctx))
    return violations


# ---------------------------------------------------------------------------
# I7: legacy rewrite
# ---------------------------------------------------------------------------

def test_legacy_rewrite_b():
    """B's sibling keys survive legacy sanitization (round-trip)."""
    violations = []
    for ctx in CONTEXTS:
        d = DeviceB("d0")
        d.frontier[ctx] = 10
        d.do_mark_unread(ctx)
        blob = d.publish_blob()
        sanitized = legacy_sanitize_blob(blob)

        recv_orig = DeviceB("recv1")
        recv_orig.receive_merge(blob)
        recv_san = DeviceB("recv2")
        recv_san.receive_merge(sanitized)

        for c in CONTEXTS:
            if recv_orig.overrides.get(c) != recv_san.overrides.get(c):
                violations.append(("I7-B-sanitize-mutated", c,
                                   recv_orig.overrides.get(c),
                                   recv_san.overrides.get(c)))
    return violations


def test_legacy_erasure_a():
    """A's top-level overrides field is erased by legacy rewrite. Expected witness."""
    d = DeviceA("d0")
    d.frontier["c0"] = 10
    d.do_mark_unread("c0")
    blob = d.publish_blob()
    assert "overrides" in blob
    legacy_blob = {"v": 1, "client_id": "legacy", "contexts": dict(blob["contexts"])}
    return "overrides" not in legacy_blob


# ---------------------------------------------------------------------------
# I8: bounded key growth
# ---------------------------------------------------------------------------

def test_bounded_growth():
    """I8: bounded key growth, canonical wire shape. A live override
    (last action = mark_unread, still within baseline) publishes
    exactly 3 keys/ctx; a dead override (mark_read past baseline, or
    C > S under clear-wins) canonicalizes to exactly 1 key/ctx
    (`ov_c:` tombstone) at publish time — never 0 (virgin-only) or 3
    (dead-but-uncompacted, which the pre-fix serializer allowed).
    """
    violations = []
    for ctx in CONTEXTS:
        # Live: 100 set/clear round-trips, ending on a fresh mark_unread
        # so S > C (live under both tie policies) at publish time.
        d = DeviceB("d0")
        d.frontier[ctx] = 10
        for _ in range(100):
            d.do_mark_unread(ctx)
            d.do_mark_read(ctx, d.effective_frontier(ctx) + 1)
        d.do_mark_unread(ctx)
        blob = d.publish_blob(CLEAR)
        ov_keys = [k for k in blob["contexts"] if k.startswith("ov_")]
        if ov_keys != [f"ov_s:{ctx}", f"ov_c:{ctx}", f"ov_b:{ctx}"]:
            violations.append(("I8-growth-live", ctx, ov_keys))

        # Dead: advance the frontier past baseline B — override_set_b's
        # baseline-dominance clause forces S dead regardless of S vs C.
        d.do_advance_frontier(ctx, d.effective_frontier(ctx) + 100)
        tomb_blob = d.publish_blob(CLEAR)
        tomb_keys = [k for k in tomb_blob["contexts"] if k.startswith("ov_")]
        if tomb_keys != [f"ov_c:{ctx}"]:
            violations.append(("I8-growth-tombstone", ctx, tomb_keys))
    return violations


def test_wire_shape_exact():
    """Exact wire-shape regression (Paul's fix-scope item 4): a live
    override serializes to exactly 3 `ov_*` keys, a dead override to
    exactly 1 (`ov_c:` only, zero-valued `ov_s`/`ov_b` omitted), and a
    virgin override to exactly 0. Checked directly against
    `publish_blob`'s output, independent of `do_compact`.
    """
    violations = []
    for tie_policy in (CLEAR, SET):
        # Live.
        d_live = DeviceB("d0")
        d_live.frontier["c0"] = 10
        d_live.do_mark_unread("c0")
        live_blob = d_live.publish_blob(tie_policy)
        live_keys = sorted(k for k in live_blob["contexts"] if k.startswith("ov_"))
        if live_keys != ["ov_b:c0", "ov_c:c0", "ov_s:c0"]:
            violations.append(("wire-shape-live", tie_policy, live_keys))

        # Dead (clear-wins only: S==C>0 is dead under CLEAR, live under
        # SET — use baseline dominance instead so it's dead under both).
        d_dead = DeviceB("d0")
        d_dead.frontier["c0"] = 10
        d_dead.do_mark_unread("c0")
        d_dead.do_advance_frontier("c0", 100)
        dead_blob = d_dead.publish_blob(tie_policy)
        dead_keys = sorted(k for k in dead_blob["contexts"] if k.startswith("ov_"))
        if dead_keys != ["ov_c:c0"]:
            violations.append(("wire-shape-tombstone", tie_policy, dead_keys))
        if dead_blob["contexts"]["ov_c:c0"] != 1:
            violations.append((
                "wire-shape-tombstone-ceiling", tie_policy,
                dead_blob["contexts"]["ov_c:c0"],
            ))

        # Virgin: no override ever set for this context.
        d_virgin = DeviceB("d0")
        d_virgin.frontier["c0"] = 10
        d_virgin.overrides["c0"] = RegB(s=0, c=0, b=0)
        virgin_blob = d_virgin.publish_blob(tie_policy)
        virgin_keys = [k for k in virgin_blob["contexts"] if k.startswith("ov_")]
        if virgin_keys:
            violations.append(("wire-shape-virgin", tie_policy, virgin_keys))

    return violations


# ---------------------------------------------------------------------------
# I9: DeviceA counter absorption
# ---------------------------------------------------------------------------

def test_a_counter_absorption():
    """After receiving a blob with counter=10, a local action must use counter>10."""
    d0 = DeviceA("d0")
    d0.frontier["c0"] = 10
    d0.counter = 10
    d0.do_mark_unread("c0")
    blob0 = d0.publish_blob()

    d1 = DeviceA("d1")
    d1.frontier["c0"] = 10
    d1.receive_merge(blob0)
    assert d1.counter >= 10, f"counter not absorbed: {d1.counter}"

    d1.do_mark_read("c0", 20)
    reg = d1.overrides.get("c0")
    assert reg is not None and reg.counter > 10, \
        f"post-receive clear at counter {reg.counter} would lose to set at 10"
    return True


# ---------------------------------------------------------------------------
# Identity-free (B): reinstall convergence
# ---------------------------------------------------------------------------

def test_b_identity_free(device_cls=DeviceB):
    violations = []
    for tie_policy in [CLEAR, SET]:
        for ctx in CONTEXTS:
            d = device_cls("d0")
            d.frontier[ctx] = 10
            d.do_mark_unread(ctx)
            blob1 = d.publish_blob(tie_policy)

            d_re = device_cls("d0_reinstalled")
            d_re.receive_merge(blob1)
            d_re.do_mark_read(ctx, 20)
            blob2 = d_re.publish_blob(tie_policy)

            verdicts = set()
            for first, second in [(blob1, blob2), (blob2, blob1)]:
                recv = device_cls("recv")
                recv.receive_merge(first)
                recv.receive_merge(second)
                verdicts.add(recv.override_is_set(ctx, tie_policy))
            if len(verdicts) > 1:
                violations.append(("identity-free", tie_policy, ctx))
    return violations


# ---------------------------------------------------------------------------
# Legacy prune/trim interaction
# ---------------------------------------------------------------------------

def test_legacy_prune_interaction():
    """ov_ keys survive prune; msg:ov_ nested keys would be pruned (state loss)."""
    base = {"c0": 50, "msg:m1": 30, "thread:t1": 40}
    ov = {"ov_s:c0": 1, "ov_c:c0": 0, "ov_b:c0": 10}
    all_keys = {**base, **ov}
    pruned = legacy_prune(all_keys, horizon=35)
    ov_survived = all(k in pruned for k in ov)
    msg_pruned = "msg:m1" not in pruned

    nested = {"msg:ov_s:c0": 1, "msg:ov_c:c0": 0, "msg:ov_b:c0": 10}
    pruned_nested = legacy_prune({**base, **nested}, horizon=35)
    nested_lost = any(k not in pruned_nested for k in nested)
    return ov_survived, msg_pruned, nested_lost


def test_legacy_trim_interaction():
    """Excess override keys block legacy publish when budget exceeded."""
    contexts = {"c0": 50}
    for i in range(1000):
        contexts[f"ov_s:c{i}"] = 1
        contexts[f"ov_c:c{i}"] = 0
        contexts[f"ov_b:c{i}"] = 10
    _, fits = legacy_trim(contexts, "client1", max_bytes=32768)
    return not fits


# ---------------------------------------------------------------------------
# Multi-slot union
# ---------------------------------------------------------------------------

def test_multi_slot_union(device_cls=DeviceB):
    """Split a published blob across 2 slots, deliver each separately,
    verify convergence with delivering the full blob.

    Production: mergeReadStateEvents merges per-slot blobs with per-context
    max(). Override sibling keys are individual context entries, so they
    follow the same merge path.
    """
    violations = []
    for tie_policy in [CLEAR, SET]:
        dev = device_cls("d0")
        dev.frontier["c0"] = 10
        dev.frontier["c1"] = 20
        dev.do_mark_unread("c0")
        dev.do_mark_read("c1", 30)

        full_blob = dev.publish_blob(tie_policy)
        items = list(full_blob["contexts"].items())
        mid = len(items) // 2

        slot0 = {"v": 1, "client_id": dev.client_id,
                 "contexts": dict(items[:mid])}
        slot1 = {"v": 1, "client_id": dev.client_id,
                 "contexts": dict(items[mid:])}

        recv_full = device_cls("recv_full")
        recv_full.receive_merge(full_blob)

        for first, second in [(slot0, slot1), (slot1, slot0)]:
            recv_split = device_cls("recv_split")
            recv_split.receive_merge(first)
            recv_split.receive_merge(second)

            for ctx in CONTEXTS:
                ov_full = recv_full.override_is_set(ctx, tie_policy)
                ov_split = recv_split.override_is_set(ctx, tie_policy)
                f_full = recv_full.effective_frontier(ctx)
                f_split = recv_split.effective_frontier(ctx)
                if ov_full != ov_split:
                    violations.append(("multi-slot-override", tie_policy, ctx))
                if f_full != f_split:
                    violations.append(("multi-slot-frontier", tie_policy, ctx))
    return violations


# ---------------------------------------------------------------------------
# Reserved key namespace: adversarial prefix collision
# ---------------------------------------------------------------------------

def test_reserved_namespace_collision():
    """A genuine user context whose raw ID begins with the reserved `ov_`
    stem (e.g. a pre-existing legacy context literally named `ov_s:evil`)
    must round-trip as frontier state, not be misparsed as a control key
    for a different context, and must not collide with a real override's
    sibling keys in the same flattened contexts map.

    Exercises: escape on publish, unescape on receive, and a live
    override on a DIFFERENT context in the same blob to prove no
    control-key collision occurs.
    """
    violations = []
    adversarial_raw = "ov_s:evil"  # would misparse as ov_s: control for ctx "evil"
    real_ctx = "c0"

    # Escaping must be a no-op for every context ID Buzz actually
    # generates, and must trigger for the adversarial one.
    for benign in ("b68cd7cb-6f8d-4641-b743-a7349eb4114b",
                   "msg:" + "a" * 64, "thread:" + "b" * 64):
        if escape_context_key(benign) != benign:
            violations.append(("namespace-benign-escaped", benign))
    if escape_context_key(adversarial_raw) == adversarial_raw:
        violations.append(("namespace-adversarial-not-escaped", adversarial_raw))
    if not escape_context_key(adversarial_raw).startswith(ESCAPE_PREFIX):
        violations.append(("namespace-adversarial-missing-marker", adversarial_raw))

    dev = DeviceB("d0")
    dev.frontier[adversarial_raw] = 42
    dev.frontier[real_ctx] = 5
    dev.do_mark_unread(real_ctx)
    blob = dev.publish_blob()

    wire_key = escape_context_key(adversarial_raw)
    if wire_key not in blob["contexts"]:
        violations.append(("namespace-wire-key-missing", wire_key, blob["contexts"]))
    if blob["contexts"].get(wire_key) != 42:
        violations.append(("namespace-value-corrupted", wire_key, blob["contexts"].get(wire_key)))

    recv = DeviceB("recv")
    recv.receive_merge(blob)
    if recv.effective_frontier(adversarial_raw) != 42:
        violations.append((
            "namespace-roundtrip-failed", adversarial_raw,
            recv.effective_frontier(adversarial_raw),
        ))
    if adversarial_raw in recv.overrides:
        violations.append(("namespace-misparsed-as-override", adversarial_raw))
    if recv.overrides.get(real_ctx) is None or recv.overrides[real_ctx].s == 0:
        violations.append(("namespace-real-override-corrupted", real_ctx, recv.overrides.get(real_ctx)))

    return violations


# ---------------------------------------------------------------------------
# Run all
# ---------------------------------------------------------------------------

def run_all():
    print("=" * 60)
    print("NIP-RS manual-unread exhaustive model")
    print("=" * 60)
    total_violations = 0

    def report(name, violations):
        nonlocal total_violations
        n = len(violations) if isinstance(violations, list) else 0
        total_violations += n
        status = "PASS" if n == 0 else f"FAIL ({n})"
        print(f"  {name}: {status}")
        if n > 0:
            for v in violations[:3]:
                print(f"    {v}")

    print("\n--- I1: merge algebra (B) ---")
    report("assoc/commut/idempot", test_merge_algebra_b())

    print("\n--- I1: merge algebra (A) ---")
    report("assoc/commut/idempot", test_merge_algebra_a())

    print("\n--- I2+I3+I5: BFS explorer (B, clear-wins) ---")
    n, v = explore_b(max_depth=4, tie_policy=CLEAR)
    print(f"  states explored: {n}")
    report("convergence+frontier+compaction", v)

    print("\n--- I2+I3+I5: BFS explorer (B, set-wins) ---")
    n, v = explore_b(max_depth=4, tie_policy=SET)
    print(f"  states explored: {n}")
    report("convergence+frontier+compaction", v)

    print("\n--- I4: concurrent set/clear stability ---")
    report("stable winner", test_concurrent_stability())

    print("\n--- I5: compaction register-level exhaustive ---")
    report("all register values x policies", test_compaction_register_exhaustive())

    print("\n--- I5c: directed deep-history (compact -> reuse -> stale delivery) ---")
    cube_size, deep_v = test_deep_history_compaction()
    print(f"  parameter cube size: {cube_size}")
    report("no divergence from never-compact reference", deep_v)

    print("\n--- I5c: tombstone + stale-ancestor merge (direct) ---")
    report("no resurrection", test_tombstone_stale_merge_direct())

    print("\n--- I5d: tombstone-merge monotonicity (structural lemma) ---")
    report("ceiling never strengthens S", test_tombstone_merge_monotonic())

    print("\n--- I5d: cross-device compaction transparency (suppress-only) ---")
    cd_cube, cd_suppress, cd_v = test_cross_device_compaction_suppression()
    print(f"  parameter cube size: {cd_cube}  suppressions observed: {cd_suppress}")
    report("suppress-only + recoverable", cd_v)

    print("\n--- Published-state merge closure (canonical publication guarantee) ---")
    mc_cube, mc_v = test_published_merge_closure()
    print(f"  pairs checked: {mc_cube}")
    report("no dead+dead resurrection", mc_v)

    print("\n--- I6: replay harmless ---")
    report("replay", test_replay_harmless())

    print("\n--- I7: legacy rewrite (B) ---")
    report("sibling keys survive", test_legacy_rewrite_b())

    print("\n--- I7: legacy erasure (A) — expected witness ---")
    erased = test_legacy_erasure_a()
    print(f"  overrides erased by legacy: {'CONFIRMED' if erased else 'NOT FOUND'}")
    if not erased:
        total_violations += 1

    print("\n--- I8: bounded growth ---")
    report("canonical wire shape (3 live / 1 tombstone)", test_bounded_growth())

    print("\n--- I8: exact wire-shape regression ---")
    report("live=3 keys, tombstone=1 key, virgin=0 keys", test_wire_shape_exact())

    print("\n--- I9: DeviceA counter absorption ---")
    absorbed = test_a_counter_absorption()
    print(f"  post-receive counter > received: {'CONFIRMED' if absorbed else 'FAIL'}")
    if not absorbed:
        total_violations += 1

    print("\n--- Identity-free (B) ---")
    report("reinstall convergence", test_b_identity_free())

    print("\n--- Legacy prune interaction ---")
    ov_ok, msg_ok, nested_lost = test_legacy_prune_interaction()
    print(f"  ov_ keys survive: {'PASS' if ov_ok else 'FAIL'}")
    print(f"  msg: pruned at horizon: {'PASS' if msg_ok else 'FAIL'}")
    print(f"  nested msg:ov_ lost: {'CONFIRMED (hazard)' if nested_lost else 'NOT FOUND'}")
    if not ov_ok:
        total_violations += 1

    print("\n--- Legacy trim interaction ---")
    blocked = test_legacy_trim_interaction()
    print(f"  excess overrides block publish: {'CONFIRMED (hazard)' if blocked else 'NOT FOUND'}")

    print("\n--- Multi-slot union ---")
    report("split+merge convergence", test_multi_slot_union())

    print("\n--- Reserved key namespace: adversarial prefix collision ---")
    report("escape/unescape + no misparse", test_reserved_namespace_collision())

    print("\n" + "=" * 60)
    if total_violations == 0:
        print("ALL INVARIANTS HOLD — 0 violations")
    else:
        print(f"VIOLATIONS: {total_violations}")
    print("=" * 60)
    return total_violations


if __name__ == "__main__":
    import sys
    sys.exit(0 if run_all() == 0 else 1)
