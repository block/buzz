------------------- MODULE RedisClusterFencingMigration -------------------
(***************************************************************************)
(* Fencing-key migration for Redis cluster mode.                          *)
(*                                                                         *)
(* The relay's tunnel session directory fences mesh frames with a          *)
(* {lease, generation} key pair per session (tunnel/directory.rs). Under   *)
(* cluster mode the pair must share a hash slot, which forces a key        *)
(* RENAME. The generation key is deliberately non-expiring and is the      *)
(* fencing authority: it must never fork (two live leases for one          *)
(* session) and never go backward or repeat (a generation, once issued,    *)
(* is never issued again for that session).                                *)
(*                                                                         *)
(* This module models the staged migration:                                *)
(*   version O (0): legacy scripts — old keys only, checks old lease.      *)
(*   version A (1): union lease check; lease written to OLD key;           *)
(*                  generation = max(oldGen,newGen)+1 written to OLD ctr   *)
(*                  (an earlier draft dual-wrote both; mutation testing    *)
(*                  proved the new-counter write redundant).               *)
(*   version B (2): union lease check; lease written to NEW key;           *)
(*                  generation = max(oldGen,newGen)+1 written to NEW.      *)
(*                  B's renewer MIGRATES any old-key lease it owns to the  *)
(*                  new key (same generation, atomic pre-cluster Lua) —    *)
(*                  this is what makes the old keyspace DRAIN by           *)
(*                  participation instead of by hope: renew is PEXPIRE,    *)
(*                  so a renewed lease never expires and no fixed wait     *)
(*                  drains anything (spec S:B2).                           *)
(*   backfill:      after full-B, fold oldGen into newGen (max-merge).     *)
(*   version C (3): new keys only (cluster-safe: no cross-slot access).    *)
(*   mode:          0 = disabled, 1 = compatible, 2 = enabled.             *)
(*                  SLOT RULES (CROSSSLOT rejection) are enforced from     *)
(*                  mode 1 onward: "one shard owns all slots" grants no    *)
(*                  cross-slot exemption (falsified live on a single-shard *)
(*                  cluster-enabled node — spec S:B1). O/A/B/migrate are   *)
(*                  cross-slot scripts, so they are guarded on mode = 0;   *)
(*                  under slot rules those scripts fail loudly and mutate  *)
(*                  nothing. 1 -> 0 is a supported revert; 1 -> 2 is the   *)
(*                  one-way door.                                          *)
(*                                                                         *)
(* ENVIRONMENT ASSUMPTION E1 (deployment gate): the phase machinery below  *)
(* — AdvancePhase requiring the whole fleet at the current phase, and      *)
(* RollbackPhase requiring the downgrade to settle before rolling back     *)
(* further — is NOT a property of Kubernetes RollingUpdate. It must be     *)
(* discharged operationally (spec §Deployment Gate): one image per phase,  *)
(* next image change blocked until all live pods report the expected       *)
(* phase and old ReplicaSets are at zero, skipped-phase rollback           *)
(* prohibited. Mutations M6–M8 show the invariants fail without E1.        *)
(*                                                                         *)
(* SCOPE OF PROOF: the model covers acquisition authority, generation      *)
(* issuance, lease migration/loss, and the mode doors. Renew (TTL          *)
(* extension) is subsumed by the untimed nondeterminism (a renewed lease   *)
(* is one where Expire has not yet fired); release is subsumed by Expire   *)
(* (both are lease loss without generation change); validate is read-only  *)
(* and its per-phase read rule is specified and tested, not modeled        *)
(* (spec §Per-phase operation table).                                      *)
(*                                                                         *)
(* One session is modeled; sessions are independent (per-session keys).    *)
(***************************************************************************)
EXTENDS Naturals, FiniteSets

CONSTANTS Pods,      \* set of relay pods
          None,      \* model value: no lease owner
          MaxGen     \* finiteness bound on generation counters

ASSUME None \notin Pods

VARIABLES
  phase,       \* deployment target version, 0..3
  ver,         \* [Pods -> 0..3], each pod's deployed script version
  oldOwner,    \* lease on OLD key: owner pod or None
  oldLeaseGen, \* generation carried by the live old-key lease (0 if none)
  newOwner,    \* lease on NEW (hash-tagged) key: owner pod or None
  newLeaseGen, \* generation carried by the live new-key lease (0 if none)
  oldGen,      \* OLD non-expiring generation counter
  newGen,      \* NEW non-expiring generation counter
  backfilled,  \* one-time oldGen -> newGen max-merge has run
  mode,        \* 0 = disabled, 1 = compatible, 2 = enabled (one-way)
  lastIssued,  \* history: highest generation ever issued to a lease
  monoOk       \* history flag: FALSE iff an issue was <= a prior issue

vars == <<phase, ver, oldOwner, oldLeaseGen, newOwner, newLeaseGen,
          oldGen, newGen, backfilled, mode, lastIssued, monoOk>>

Max(a, b) == IF a > b THEN a ELSE b

TypeOK ==
  /\ phase \in 0..3
  /\ ver \in [Pods -> 0..3]
  /\ oldOwner \in Pods \cup {None}
  /\ newOwner \in Pods \cup {None}
  /\ oldLeaseGen \in 0..MaxGen
  /\ newLeaseGen \in 0..MaxGen
  /\ oldGen \in 0..MaxGen
  /\ newGen \in 0..MaxGen
  /\ backfilled \in BOOLEAN
  /\ mode \in 0..2
  /\ lastIssued \in 0..MaxGen
  /\ monoOk \in BOOLEAN

Init ==
  /\ phase = 0
  /\ ver = [p \in Pods |-> 0]
  /\ oldOwner = None /\ oldLeaseGen = 0
  /\ newOwner = None /\ newLeaseGen = 0
  /\ oldGen = 0 /\ newGen = 0
  /\ backfilled = FALSE
  /\ mode = 0
  /\ lastIssued = 0
  /\ monoOk = TRUE

(***************************************************************************)
(* Deployment machinery (environment assumption E1 — see header).          *)
(***************************************************************************)

\* The fleet may only chase the current phase; a phase only advances when
\* every pod has reached it. Gate into C (phase 3): backfill has run AND no
\* live old-key lease remains. The drain conjunct is participation-based
\* (B migrates owned old-key leases; orphaned leases expire by TTL), NOT a
\* fixed wait — renew resets the TTL, so "wait one TTL" drains nothing.
AdvancePhase ==
  /\ phase < 3
  /\ \A p \in Pods : ver[p] = phase
  /\ (phase = 2) => (backfilled /\ oldOwner = None)
  /\ phase' = phase + 1
  /\ UNCHANGED <<ver, oldOwner, oldLeaseGen, newOwner, newLeaseGen,
                 oldGen, newGen, backfilled, mode, lastIssued, monoOk>>

UpgradePod(p) ==
  /\ ver[p] < phase
  /\ ver' = [ver EXCEPT ![p] = phase]
  /\ UNCHANGED <<phase, oldOwner, oldLeaseGen, newOwner, newLeaseGen,
                 oldGen, newGen, backfilled, mode, lastIssued, monoOk>>

DowngradePod(p) ==
  /\ ver[p] > phase
  /\ ver' = [ver EXCEPT ![p] = phase]
  /\ UNCHANGED <<phase, oldOwner, oldLeaseGen, newOwner, newLeaseGen,
                 oldGen, newGen, backfilled, mode, lastIssued, monoOk>>

\* Emergency rollback: one adjacent phase at a time, only in disabled mode
\* (from compatible, revert the mode first), never past a running
\* downgrade (no pod may still be above the current target — dropping
\* this guard is mutation M8). Rolling back into O (phase 0) additionally
\* requires the new keyspace to be lease-drained (dropping this is M6) and
\* re-syncs the old counter by reverse max-merge: O issues from the old
\* counter alone, so without the merge it re-issues generations B already
\* issued (dropping the merge is M7). Every rollback conservatively
\* invalidates the backfill; it must re-run before the C-gate can pass.
RollbackPhase ==
  /\ phase > 0
  /\ mode = 0
  /\ \A p \in Pods : ver[p] <= phase
  /\ IF phase = 1
       THEN /\ newOwner = None
            /\ oldGen' = Max(oldGen, newGen)
       ELSE UNCHANGED oldGen
  /\ phase' = phase - 1
  /\ backfilled' = FALSE
  /\ UNCHANGED <<ver, oldOwner, oldLeaseGen, newOwner, newLeaseGen,
                 newGen, mode, lastIssued, monoOk>>

\* One-time backfill after the whole fleet is on B: fold the old counter
\* into the new one. Idempotent max-merge.
Backfill ==
  /\ phase = 2
  /\ \A p \in Pods : ver[p] = 2
  /\ newGen' = Max(newGen, oldGen)
  /\ backfilled' = TRUE
  /\ UNCHANGED <<phase, ver, oldOwner, oldLeaseGen, newOwner, newLeaseGen,
                 oldGen, mode, lastIssued, monoOk>>

(***************************************************************************)
(* Mode doors. Slot rules turn on at compatible (S:B1), so the whole      *)
(* fleet must already run C — the only slot-clean script version.          *)
(* compatible -> disabled is the supported revert; enabled is one-way.     *)
(***************************************************************************)

EnterCompatible ==
  /\ mode = 0
  /\ phase = 3
  /\ \A p \in Pods : ver[p] = 3
  /\ mode' = 1
  /\ UNCHANGED <<phase, ver, oldOwner, oldLeaseGen, newOwner, newLeaseGen,
                 oldGen, newGen, backfilled, lastIssued, monoOk>>

RevertCompatible ==
  /\ mode = 1
  /\ mode' = 0
  /\ UNCHANGED <<phase, ver, oldOwner, oldLeaseGen, newOwner, newLeaseGen,
                 oldGen, newGen, backfilled, lastIssued, monoOk>>

EnableCluster ==
  /\ mode = 1
  /\ mode' = 2
  /\ UNCHANGED <<phase, ver, oldOwner, oldLeaseGen, newOwner, newLeaseGen,
                 oldGen, newGen, backfilled, lastIssued, monoOk>>

(***************************************************************************)
(* Acquire, per script version. Each is one atomic Lua script. A, B and   *)
(* the B-migrate touch both keyspaces; O touches only the old pair, but    *)
(* that pair is itself cross-slot (un-tagged lease + generation keys).     *)
(* All four are therefore guarded on mode = 0: under slot rules they fail  *)
(* loudly and mutate nothing, which the guard models by absence.           *)
(***************************************************************************)

RecordIssue(g) ==
  /\ lastIssued' = Max(lastIssued, g)
  /\ monoOk' = (monoOk /\ (g > lastIssued))

\* Version O (legacy, deployed today): old keys only.
AcquireO(p) ==
  /\ ver[p] = 0
  /\ mode = 0
  /\ oldOwner = None
  /\ oldGen + 1 <= MaxGen
  /\ oldGen' = oldGen + 1
  /\ oldOwner' = p /\ oldLeaseGen' = oldGen + 1
  /\ RecordIssue(oldGen + 1)
  /\ UNCHANGED <<phase, ver, newOwner, newLeaseGen, newGen, backfilled,
                 mode>>

\* Version A: union lease check; lease still on OLD key; generation is
\* max-merged over BOTH counters but written only to the OLD one. (An
\* earlier draft dual-wrote the generation to both counters; mutation
\* testing showed the dual-write redundant — the mutant removing it
\* passes the full state space, because B's union max-merge read and the
\* backfill already carry A's issues forward. The max-merge READ is the
\* load-bearing part: dropping it is mutation M5.)
AcquireA(p) ==
  /\ ver[p] = 1
  /\ mode = 0
  /\ oldOwner = None /\ newOwner = None
  /\ LET g == Max(oldGen, newGen) + 1 IN
       /\ g <= MaxGen
       /\ oldGen' = g
       /\ oldOwner' = p /\ oldLeaseGen' = g
       /\ RecordIssue(g)
  /\ UNCHANGED <<phase, ver, newOwner, newLeaseGen, newGen, backfilled,
                 mode>>

\* Version B: union lease check; lease moves to the NEW key; generation
\* max-merged into the NEW counter.
AcquireB(p) ==
  /\ ver[p] = 2
  /\ mode = 0
  /\ oldOwner = None /\ newOwner = None
  /\ LET g == Max(oldGen, newGen) + 1 IN
       /\ g <= MaxGen
       /\ newGen' = g
       /\ newOwner' = p /\ newLeaseGen' = g
       /\ RecordIssue(g)
  /\ UNCHANGED <<phase, ver, oldOwner, oldLeaseGen, oldGen, backfilled,
                 mode>>

\* Version B renew-migrate: a B-pod that still owns an old-key lease
\* (acquired before its upgrade) moves it to the new key at its next renew
\* tick — same owner, SAME generation (no new issuance; monotonicity is
\* untouched), atomic cross-slot Lua, legal only in disabled mode. The
\* script requires the new key empty; the counter is max-merged so the
\* moved lease stays grounded. This action is what discharges the C-gate's
\* oldOwner = None conjunct by participation.
MigrateB(p) ==
  /\ ver[p] = 2
  /\ mode = 0
  /\ oldOwner = p
  /\ newOwner = None
  /\ newOwner' = p /\ newLeaseGen' = oldLeaseGen
  /\ newGen' = Max(newGen, oldLeaseGen)
  /\ oldOwner' = None /\ oldLeaseGen' = 0
  /\ UNCHANGED <<phase, ver, oldGen, backfilled, mode, lastIssued, monoOk>>

\* Version C: new keys only. Slot-clean; runs in every mode.
AcquireC(p) ==
  /\ ver[p] = 3
  /\ newOwner = None
  /\ newGen + 1 <= MaxGen
  /\ newGen' = newGen + 1
  /\ newOwner' = p /\ newLeaseGen' = newGen + 1
  /\ RecordIssue(newGen + 1)
  /\ UNCHANGED <<phase, ver, oldOwner, oldLeaseGen, oldGen, backfilled,
                 mode>>

(***************************************************************************)
(* Lease loss: TTL expiry or explicit release (same state effect).         *)
(* Generations survive.                                                    *)
(***************************************************************************)

ExpireOld ==
  /\ oldOwner /= None
  /\ oldOwner' = None /\ oldLeaseGen' = 0
  /\ UNCHANGED <<phase, ver, newOwner, newLeaseGen, oldGen, newGen,
                 backfilled, mode, lastIssued, monoOk>>

ExpireNew ==
  /\ newOwner /= None
  /\ newOwner' = None /\ newLeaseGen' = 0
  /\ UNCHANGED <<phase, ver, oldOwner, oldLeaseGen, oldGen, newGen,
                 backfilled, mode, lastIssued, monoOk>>

Next ==
  \/ AdvancePhase
  \/ RollbackPhase
  \/ Backfill
  \/ EnterCompatible
  \/ RevertCompatible
  \/ EnableCluster
  \/ ExpireOld
  \/ ExpireNew
  \/ \E p \in Pods : UpgradePod(p) \/ DowngradePod(p)
                     \/ AcquireO(p) \/ AcquireA(p) \/ AcquireB(p)
                     \/ MigrateB(p) \/ AcquireC(p)

Spec == Init /\ [][Next]_vars

(***************************************************************************)
(* Safety invariants.                                                      *)
(***************************************************************************)

\* T1: fencing authority never forks — at most one live lease per session
\* across BOTH keyspaces, at every instant of the migration, including
\* rollbacks.
Inv_SingleAuthority == oldOwner = None \/ newOwner = None

\* T2: generations are strictly monotonic per session — no generation is
\* ever issued twice, across the old/new counter handoff and across
\* rollback re-merges.
Inv_Monotonic == monoOk

\* T3: slot rules (compatible or enabled) are only ever on with the old
\* keyspace fully drained and the whole fleet on slot-clean scripts —
\* so no post-door execution ever attempts a cross-slot fencing script,
\* and any residual old-key state is inert garbage.
Inv_SlotRulesSafe ==
  (mode >= 1) => /\ backfilled
                 /\ oldOwner = None
                 /\ phase = 3
                 /\ \A p \in Pods : ver[p] = 3

\* The door is one-way: this is an axiom of the environment (A3), recorded
\* as an action guard (no transition leaves mode = 2), not an invariant.

\* Sanity: a live lease always carries a generation its counter dominates.
Inv_LeaseGenGrounded ==
  /\ (oldOwner /= None) => (oldLeaseGen > 0 /\ oldLeaseGen <= oldGen)
  /\ (newOwner /= None) => (newLeaseGen > 0 /\ newLeaseGen <= newGen)

(***************************************************************************)
(* Drain-reachability probe (spec S:B2 defense; found in review round 2).  *)
(*                                                                         *)
(* The safety model above lets ExpireOld/ExpireNew fire unguarded — the    *)
(* adversarial worst case for SAFETY (leases may vanish at any moment).    *)
(* But for DRAIN it is the friendly best case: an old-key lease whose      *)
(* owner is alive and renewing never expires (renew is PEXPIRE — the exact *)
(* premise B2 falsified), so unguarded expiry lets the C-gate's            *)
(* oldOwner = None conjunct be discharged by luck. Consequence: deleting   *)
(* MigrateB from Next leaves every safety invariant green — the model      *)
(* could not tell the B2 fix from its absence (found by Dawn, round 2).    *)
(*                                                                         *)
(* This probe closes that hole. DrainInit is the stall state: fleet fully  *)
(* on B, backfill done, one live B-pod still owning an old-key lease.      *)
(* DrainNext is Next WITHOUT spontaneous expiry — faithful to a lease      *)
(* whose owner never crashes. Probe_NotC is checked as an "invariant"      *)
(* with INVERTED verdict semantics: TLC reporting Probe_NotC VIOLATED      *)
(* proves phase C is REACHABLE from the stall state (pass); TLC GREEN      *)
(* means the migration deadlocks at the C-gate forever (fail). Run with    *)
(* RedisClusterFencingMigrationDrain.cfg. Mutant M10 (drop MigrateB from   *)
(* DrainNext = round-1 behavior, B renews the old key instead of           *)
(* migrating) must leave this probe green — proving MigrateB is the only   *)
(* mechanism that discharges the gate without luck.                        *)
(***************************************************************************)

DrainInit ==
  /\ phase = 2
  /\ ver = [p \in Pods |-> 2]
  /\ oldOwner = CHOOSE p \in Pods : TRUE
  /\ oldLeaseGen = 1
  /\ newOwner = None /\ newLeaseGen = 0
  /\ oldGen = 1 /\ newGen = 1
  /\ backfilled = TRUE
  /\ mode = 0
  /\ lastIssued = 1
  /\ monoOk = TRUE

DrainNext ==
  \/ AdvancePhase
  \/ RollbackPhase
  \/ Backfill
  \/ EnterCompatible
  \/ RevertCompatible
  \/ EnableCluster
  \/ \E p \in Pods : UpgradePod(p) \/ DowngradePod(p)
                     \/ AcquireO(p) \/ AcquireA(p) \/ AcquireB(p)
                     \/ AcquireC(p)
                     \/ MigrateB(p)  \* the B2 mechanism under test (M10)

DrainSpec == DrainInit /\ [][DrainNext]_vars

\* INVERTED verdict: a violation is the PASS (C reachable by participation).
Probe_NotC == phase /= 3

=============================================================================
