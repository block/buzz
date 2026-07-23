"""Mutation test: prove the acceptance model has TEETH.

We inject the two most tempting spec-weakenings and confirm the model CATCHES
each one. A model that stays green under a real weakening is worthless.

  M1: accept on generation ALONE (drop NIP-01 ordering, the (a) clause).
      => the poison event e3 (gen9, created150 < tombstone's 200) is accepted,
         resurrecting the lease and poisoning the watermark. Must trip I1 & I3.

  M2: accept on NIP-01 ordering ALONE (drop the generation watermark, clause (b)).
      => a high-created_at REPLAY with a stale generation wins; watermark is no
         longer the resurrection backstop. Must trip a resurrection under replay.
"""
from itertools import permutations as P
from acceptance import Ev, nip01_beats

def run(mode):
    universe = [
        Ev("e1", 1, 100, True),
        Ev("e2", 2, 200, False),   # legit revoke
        Ev("e3", 9, 150, True),    # poison: high gen, loses NIP-01
        Ev("e4", 3, 250, True),
        Ev("e5", 1, 100, True),
        Ev("a1", 5, 200, True),
        # WITNESS for clause (b): an event with the HIGHEST created_at but a STALE
        # generation. NIP-01 alone accepts it (created 300 > all); only the
        # generation watermark rejects it. This is the "malicious high-created_at
        # replay with stale gen" the watermark exists to stop.
        Ev("z1", 0, 300, True),
    ]
    caught = 0
    for perm in P(universe):
        stored = None; effective = False; watermark = -1
        for ev in perm:
            wins_nip01 = nip01_beats(ev, stored)
            wins_gen = ev.gen > watermark
            if mode == "M1":      # gen only
                accept = wins_gen
            elif mode == "M2":    # nip01 only
                accept = wins_nip01
            else:                 # spec: both
                accept = wins_nip01 and wins_gen
            eff_before = effective
            wm_before = watermark
            if accept:
                stored = ev; effective = ev.active; watermark = ev.gen
            # resurrection check: tombstone stored, then flipped active by an event
            # that does NOT beat both orderings
            if effective and not eff_before and stored is ev:
                if not (nip01_beats(ev, None) and ev.gen > wm_before):
                    pass
            # poison check: nip01-loser raised watermark
            if not nip01_beats(ev, stored if stored is not ev else None):
                pass
        # simpler post-hoc: did e3 (the poison) ever end up as the effective active
        # state after e2's tombstone appeared earlier in the order?
        # replay to detect
        st=None; ef=False; wm=-1; tomb=False; bug=False
        for ev in perm:
            wn = nip01_beats(ev, st); wg = ev.gen > wm
            acc = wg if mode=="M1" else (wn if mode=="M2" else (wn and wg))
            if acc:
                st=ev; ef=ev.active; wm=ev.gen
            if st is not None and not st.active and not ef:
                tomb=True
            if tomb and ef and st is ev and ev in (universe[2], universe[4], universe[6]):
                # e3, e5, or z1 -- none should EVER be the effective active state
                # after e2's tombstone. e3/e5 lose NIP-01; z1 loses only on gen
                # (stale generation) -- so z1 is the pure clause-(b) witness.
                bug=True
        if bug:
            caught += 1
    return caught

for m, desc in [("M1","gen-only (drop NIP-01)"), ("M2","nip01-only (drop watermark)"), ("SPEC","dual-ordering (spec)")]:
    c = run(m)
    print(f"{m:5} {desc:28} -> bug orderings detected: {c}")
