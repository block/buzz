------------------------- MODULE GitGovernanceLand -------------------------
(***************************************************************************)
(* How a governed push takes effect.  GitGovernance.tla accepts a push in *)
(* one step, against the authority in force at that step, and publishes   *)
(* it in a later one.  The repository state publication changes is a      *)
(* pointer (GitOnObjectStore.tla), and the authority lives in Postgres.    *)
(* This module asks which commit protocols keep those steps honest.        *)
(*                                                                         *)
(* Two protocols are modeled.                                              *)
(*                                                                         *)
(*   "lock"  The relay takes a shared lock on the authority rows, judges   *)
(*           the push, sends a compare-and-swap of the object-store        *)
(*           pointer, waits for the answer, records the receipt and        *)
(*           releases the lock.  Authority writes wait for the lock.       *)
(*                                                                         *)
(*   "row"   The pointer is a Postgres row.  One transaction reads the     *)
(*           authority under a shared lock on a per-repo authority row     *)
(*           that every authority write updates, and notes the pointer it  *)
(*           read.  One statement then swaps the pointer row only if it    *)
(*           still holds that value and the clock has not reached the      *)
(*           grant's expiry; it holds the row until the transaction        *)
(*           writes the receipt and commits.  Authority writes wait for    *)
(*           the shared lock.                                              *)
(*                                                                         *)
(* Where the decision takes effect.  The refs become visible at COMMIT,    *)
(* and that is where a push linearizes in the sense of GitRefDB.tla.  The  *)
(* authority is judged at the swap: no authority write can commit between  *)
(* the swap and the commit, but the clock can pass the expiry there.  The  *)
(* model allows that, and check.sh shows it is reachable                   *)
(* ("commit-after-expiry"), so the promise is stated as it holds: a land's *)
(* write was accepted by the store before its grant expired, and no        *)
(* revocation committed before the land did.  The interval is one         *)
(* statement and the receipt insert, inside the database's transaction    *)
(* timeout.  Under "lock" the store accepts a write with no check at all,  *)
(* whenever the request arrives, which may be after its sender died.       *)
(*                                                                         *)
(* How this composes with the other modules.  MCGitGovernanceLand.tla    *)
(* checks that protocol "row" refines GitGovernance.tla, whose Accept is   *)
(* the swap and whose Publish is the commit, and GitRefDB.tla, whose      *)
(* Linearize is the commit; and that the promises of GitGovernance hold of *)
(* it under that mapping.  The pointer here is a single value: a push's    *)
(* ref updates are one pointer swap.                                       *)
(*                                                                         *)
(* What the model lets happen, because it happens: a relay process dies at *)
(* any point, and its database session, locks and uncommitted writes die  *)
(* with it; a request already sent to the object store is still applied,   *)
(* later; and authority lapses with no write at all, when a grant expires. *)
(*                                                                         *)
(* docs/spec/check.sh shows that "row" keeps the promises below under all  *)
(* of that, and that "lock" keeps them only if processes never die and     *)
(* grants never expire: each of those alone breaks it.                     *)
(***************************************************************************)
EXTENDS Naturals, FiniteSets

CONSTANTS
    Procs,      \* relay processes, each landing one push: a set of positive
                \* numbers; p installs the value p, and the pointer starts at 0
    Protocol,   \* "lock" or "row"
    MayCrash,   \* processes may die
    Expiry,     \* the clock reading at which the pusher's grant lapses
    MaxClock,
    Faults      \* seeded errors; {} is the design

VARIABLES
    clock,      \* time; authority can lapse as it advances
    revoked,    \* an authority write (role removed, key banned) committed
    locks,      \* processes holding the shared lock on the authority
    rowLock,    \* the process whose uncommitted swap holds the pointer row, or 0
    pc,         \* pc[p]: "idle", "locked", "sent", "judged", "swapped",
                \*        "refusing", "done", "dead"
    judged,     \* judged[p]: the verdict p's read reached (protocol "row")
    seen,       \* seen[p]: the pointer value p's write names as the old one
    inTime,     \* inTime[p]: the store accepted p's write before the expiry
    inflight,   \* compare-and-swap requests sent to the object store
    answer,     \* answer[p]: "none", "ok", "lost"
    pointer,    \* the committed repository state
    history,    \* every value the pointer has held
    receipts    \* the record: [seq, p, ok], whether p's push landed

Init0 == 0
NoProc == 0
Target == [p \in Procs |-> p]

vars == <<clock, revoked, locks, rowLock, pc, judged, seen, inTime,
          inflight, answer, pointer, history, receipts>>

\* The authority the push needs holds now.
AllowedNow == ~revoked /\ clock < Expiry

Init ==
    /\ clock = 0 /\ revoked = FALSE /\ locks = {} /\ rowLock = NoProc
    /\ pc = [p \in Procs |-> "idle"] /\ judged = [p \in Procs |-> FALSE]
    /\ seen = [p \in Procs |-> Init0] /\ inTime = [p \in Procs |-> FALSE]
    /\ inflight = {} /\ answer = [p \in Procs |-> "none"]
    /\ pointer = Init0 /\ history = {Init0} /\ receipts = {}

Tick ==
    /\ clock < MaxClock /\ clock' = clock + 1
    /\ UNCHANGED <<revoked, locks, rowLock, pc, judged, seen, inTime,
                   inflight, answer, pointer, history, receipts>>

\* An authority write.  It waits for every shared lock.
Revoke ==
    /\ ~revoked /\ locks = {}
    /\ revoked' = TRUE
    /\ UNCHANGED <<clock, locks, rowLock, pc, judged, seen, inTime,
                   inflight, answer, pointer, history, receipts>>

\* A process dies.  Its session dies with it: its locks are released and
\* its uncommitted writes are rolled back.  What it has sent stays sent.
Crash(p) ==
    /\ MayCrash /\ pc[p] \notin {"done", "dead"}
    /\ pc' = [pc EXCEPT ![p] = "dead"] /\ locks' = locks \ {p}
    /\ rowLock' = IF rowLock = p THEN NoProc ELSE rowLock
    /\ UNCHANGED <<clock, revoked, judged, seen, inTime,
                   inflight, answer, pointer, history, receipts>>

Finish(p) == pc' = [pc EXCEPT ![p] = "done"]

Receipt(p, ok) == [seq |-> Cardinality(receipts), p |-> p, ok |-> ok]
Landed == {x.p : x \in {x \in receipts : x.ok}}

---------------------------------------------------------------------------
\* Protocol "lock".

Lock(p) ==
    /\ Protocol = "lock" /\ pc[p] = "idle"
    /\ locks' = locks \cup {p} /\ pc' = [pc EXCEPT ![p] = "locked"]
    /\ UNCHANGED <<clock, revoked, rowLock, judged, seen, inTime,
                   inflight, answer, pointer, history, receipts>>

Judge(p) ==
    /\ pc[p] = "locked"
    /\ IF AllowedNow
       THEN /\ inflight' = inflight \cup {[p |-> p, old |-> pointer, new |-> Target[p]]}
            /\ seen' = [seen EXCEPT ![p] = pointer]
            /\ pc' = [pc EXCEPT ![p] = "sent"]
            /\ locks' = locks
       ELSE /\ Finish(p) /\ locks' = locks \ {p}
            /\ UNCHANGED <<inflight, seen>>
    /\ UNCHANGED <<clock, revoked, rowLock, judged, inTime,
                   answer, pointer, history, receipts>>

\* The object store applies a request, whether or not its sender lives.  It
\* checks the old value, and nothing else.
Arrive(m) ==
    /\ m \in inflight
    /\ inflight' = inflight \ {m}
    /\ IF pointer = m.old
       THEN /\ pointer' = m.new /\ history' = history \cup {m.new}
            /\ inTime' = [inTime EXCEPT ![m.p] = clock < Expiry]
            /\ answer' = [answer EXCEPT ![m.p] = "ok"]
       ELSE /\ answer' = [answer EXCEPT ![m.p] = "lost"]
            /\ UNCHANGED <<pointer, history, inTime>>
    /\ UNCHANGED <<clock, revoked, locks, rowLock, pc, judged, seen, receipts>>

Record(p) ==
    /\ pc[p] = "sent" /\ answer[p] # "none"
    /\ receipts' = IF answer[p] = "ok" THEN receipts \cup {Receipt(p, TRUE)} ELSE receipts
    /\ Finish(p) /\ locks' = locks \ {p}
    /\ UNCHANGED <<clock, revoked, rowLock, judged, seen, inTime,
                   inflight, answer, pointer, history>>

---------------------------------------------------------------------------
\* Protocol "row", one transaction in three steps.
\*   RowRead  begins it, reads the authority under the shared lock, judges
\*            the push and notes the pointer it read.
\*   RowSwap  is the statement UPDATE ... SET the new value WHERE the value
\*            is the one read AND clock_timestamp() is before the expiry.
\*            It waits for the row while another uncommitted swap holds it,
\*            then evaluates its condition against the committed value.
\*   RowEnd   if the swap matched, writes the receipt and commits: the
\*            pointer takes the new value and both locks are released.
\*   RowRefuse if it did not, writes a refusal receipt and commits.
\* A process that dies before it commits leaves nothing behind.  Seeded errors:
\*   "unlocked-read"   the read takes no lock, so an authority write can
\*                     commit before the land does (true of every isolation
\*                     level: a serializable order with the land first
\*                     exists, though the write was reported first);
\*   "expiry-at-read"  the swap trusts the verdict of the read and does
\*                     not look at the clock;
\*   "blind-swap"      the swap does not require the value it read.

RowRead(p) ==
    /\ Protocol = "row" /\ pc[p] = "idle"
    /\ judged' = [judged EXCEPT ![p] = AllowedNow]
    /\ seen' = [seen EXCEPT ![p] = pointer]
    /\ locks' = IF "unlocked-read" \in Faults THEN locks ELSE locks \cup {p}
    /\ pc' = [pc EXCEPT ![p] = "judged"]
    /\ UNCHANGED <<clock, revoked, rowLock, inTime,
                   inflight, answer, pointer, history, receipts>>

RowSwap(p) ==
    /\ Protocol = "row" /\ pc[p] = "judged" /\ rowLock = NoProc
    /\ IF /\ judged[p]
          /\ pointer = seen[p] \/ "blind-swap" \in Faults
          /\ clock < Expiry \/ "expiry-at-read" \in Faults
       THEN /\ rowLock' = p /\ pc' = [pc EXCEPT ![p] = "swapped"]
            /\ inTime' = [inTime EXCEPT ![p] = clock < Expiry]
            /\ locks' = locks
       ELSE /\ pc' = [pc EXCEPT ![p] = "refusing"]
            /\ UNCHANGED <<locks, rowLock, inTime>>
    /\ UNCHANGED <<clock, revoked, judged, seen,
                   inflight, answer, pointer, history, receipts>>

RowEnd(p) ==
    /\ pc[p] = "swapped"
    /\ pointer' = Target[p] /\ history' = history \cup {Target[p]}
    /\ receipts' = receipts \cup {Receipt(p, TRUE)}
    /\ rowLock' = NoProc /\ locks' = locks \ {p}
    /\ Finish(p)
    /\ UNCHANGED <<clock, revoked, judged, seen, inTime, inflight, answer>>

RowRefuse(p) ==
    /\ pc[p] = "refusing"
    /\ receipts' = receipts \cup {Receipt(p, FALSE)}
    /\ locks' = locks \ {p}
    /\ Finish(p)
    /\ UNCHANGED <<clock, revoked, rowLock, judged, seen, inTime,
                   inflight, answer, pointer, history>>

Next ==
    \/ Tick \/ Revoke
    \/ \E p \in Procs : \/ Crash(p) \/ Lock(p) \/ Judge(p) \/ Record(p)
                        \/ RowRead(p) \/ RowSwap(p) \/ RowEnd(p) \/ RowRefuse(p)
    \/ \E m \in inflight : Arrive(m)

Spec == Init /\ [][Next]_vars

---------------------------------------------------------------------------
\* When the pointer moves, no authority write has committed, and the store
\* accepted the write that moves it before the grant expired.
LandsUnderCurrentAuthority ==
    [][pointer' # pointer => ~revoked /\ inTime'[pointer']]_vars

\* The pointer moves only by a write naming its current value as the old
\* one: no land overwrites another it did not see.
NoLostUpdate ==
    [][pointer' # pointer =>
          \E p \in Procs : seen[p] = pointer /\ Target[p] = pointer']_vars

\* Every value the pointer took is in the record, or a living process is
\* still about to record it.
EveryLandRecorded ==
    \A v \in history \ {Init0} :
        v \in Landed \/ \E p \in Procs : pc[p] = "sent" /\ Target[p] = v

\* Coverage: a push lands.
NeverLands == pointer = Init0

\* Coverage: a land can commit after its grant expired, once its swap was
\* accepted in time.  This is the interval the header describes.
CommitsBeforeExpiry == [][pointer' # pointer => clock < Expiry]_vars
=============================================================================
