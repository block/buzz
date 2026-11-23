------------------------ MODULE MCGitGovernanceLand ------------------------
(***************************************************************************)
(* The row protocol of GitGovernanceLand.tla refines GitGovernance.tla and *)
(* GitRefDB.tla.  TLC checks both mappings (GovRefined, RefDBRefined) on   *)
(* every behavior of the protocol, crashes and expiry included, together   *)
(* with GitGovernance's promises under the first mapping, so the promises  *)
(* hold of the protocol and not just of the abstract decision.             *)
(*                                                                         *)
(* The cast is the smallest one in which the protocol's choices matter:    *)
(* one repo r with one branch, an admin a, an owner b, and two agents of   *)
(* b, one per relay process, each pushing its own commit under a grant     *)
(* that expires when the clock reaches Expiry.  The authority write the    *)
(* protocol calls Revoke is a's ban of b, which ends both agents'          *)
(* authority.  Pointer value v is commit C[v]; 0 is the empty repo.        *)
(*                                                                         *)
(*   protocol step          GitGovernance       GitRefDB                   *)
(*   -------------          -------------       --------                   *)
(*   (the repo is created)  -                   Create                     *)
(*   RowRead                Propose             Invoke                     *)
(*   RowSwap, matched       Accept, allowed     -                          *)
(*   RowSwap, unmatched     -                   -                          *)
(*   RowEnd (COMMIT)        Publish             Linearize, applied         *)
(*   RowRefuse (COMMIT)     Accept, refused     Linearize, rejected        *)
(*   crash after the swap   Abort               -                          *)
(*   crash before it        Drop                -                          *)
(*   Revoke                 Ban                 -                          *)
(*   Tick to Expiry         Expire              -                          *)
(*                                                                         *)
(* So the authority is judged at the swap and the refs move at COMMIT, the *)
(* two steps GitGovernance separates.  An unmatched swap is refused at its *)
(* COMMIT, which is sound because nothing it was refused for can be undone *)
(* in between: an authority write waits for its lock, and expiry and a     *)
(* moved pointer are permanent.                                            *)
(*                                                                         *)
(* Two history variables are added, and they constrain nothing: created,  *)
(* so that the check starts from GitRefDB's initial state and the first    *)
(* step creates the repo, and step, the GitGovernance step just taken      *)
(* (that module's act).  invoked records which processes have begun.      *)
(*                                                                         *)
(* Assumptions outside this check: the objects a push names are stored     *)
(* and closed before its swap (GitOnObjectStore.tla checks that); a client *)
(* is answered only after its transaction commits, with the outcome        *)
(* GitOnObjectStore.tla gives it; and HEAD changes, like every authority   *)
(* write, go through the same fence.                                       *)
(***************************************************************************)
EXTENDS GitGovernanceLand, TLC

CONSTANTS a, b, g1, g2, ch, r, main, k1, k2, NoCommit, None,
          Absent, NoCall, NoOut, NoCmd, NoResp, Error, NoEntry, Manager

VARIABLES created, step, invoked

ASSUME Procs = {1, 2} /\ Protocol = "row"

Agent == [p \in Procs |-> IF p = 1 THEN g1 ELSE g2]
C == [v \in {0} \cup Procs |-> CASE v = 1 -> k1 [] v = 2 -> k2 [] OTHER -> NoCommit]
Keys == {a, b, g1, g2}

Cmds(p) == {[ref |-> main, old |-> C[seen[p]], new |-> C[p]]}
Push(p) == [by |-> Agent[p], repo |-> r, cmds |-> Cmds(p), pre |-> FALSE]
Rec(p, ok) ==
    [by |-> Agent[p], for |-> b, repo |-> r, cmds |-> Cmds(p), ok |-> ok,
     rules |-> {<<main, {}>>}, evidence |-> {}, checks |-> {}]
Stamped(x) ==
    [seq |-> x.seq, by |-> Agent[x.p], for |-> b, repo |-> r, cmds |-> Cmds(x.p),
     ok |-> x.ok, rules |-> {<<main, {}>>}, evidence |-> {}, checks |-> {}]

-----------------------------------------------------------------------------
\* The protocol, with the history variables.

mcVars == <<vars, created, step, invoked>>

MCInit == Init /\ created = FALSE /\ step = [kind |-> "init"] /\ invoked = {}

Was(x) == step' = x
Same == UNCHANGED step

MCNext ==
    \/ ~created /\ created' = TRUE /\ UNCHANGED <<vars, step, invoked>>
    \/ /\ created /\ UNCHANGED created
       /\ \/ /\ Tick /\ UNCHANGED invoked
             /\ IF clock' = Expiry THEN Was([kind |-> "expire"]) ELSE Same
          \/ Revoke /\ Was([kind |-> "ban", by |-> a]) /\ UNCHANGED invoked
          \/ \E p \in Procs :
               \/ /\ Crash(p) /\ UNCHANGED invoked
                  /\ CASE pc[p] = "swapped" -> Was([kind |-> "abort", p |-> Push(p)])
                       [] pc[p] \in {"judged", "refusing"} -> Was([kind |-> "drop", p |-> Push(p)])
                       [] OTHER -> Same
               \/ RowRead(p) /\ Was([kind |-> "push"]) /\ invoked' = invoked \cup {p}
               \/ /\ RowSwap(p) /\ UNCHANGED invoked
                  /\ IF pc'[p] = "swapped"
                     THEN Was([kind |-> "accept", p |-> Push(p), ok |-> TRUE]) ELSE Same
               \/ RowEnd(p) /\ Was([kind |-> "publish", p |-> Push(p)]) /\ UNCHANGED invoked
               \/ /\ RowRefuse(p) /\ UNCHANGED invoked
                  /\ Was([kind |-> "accept", p |-> Push(p), ok |-> FALSE])

MCSpec == MCInit /\ [][MCNext]_mcVars

-----------------------------------------------------------------------------
\* The mapping onto GitGovernance.

Landed1 == {v \in history : v # Init0}
DocIdsHere == {<<"floor", None>>, <<"trusted", None>>}
                \cup {<<t, r>> : t \in {"labels", "rules", "bind", "head"}}
GovDocs == [d \in DocIdsHere |-> CASE d[1] = "bind" -> "bind"
                                   [] d[1] = "head" -> "main"
                                   [] OTHER -> "none"]
Owned == [k \in Keys |-> IF k = a THEN "admin" ELSE "member"]

Gov == INSTANCE GitGovernance WITH
    People <- {a, b}, Agents <- {g1, g2}, Channels <- {ch}, Repos <- {r},
    RepoKey <- [x \in {r} |-> b], Branches <- {main}, Tags <- {},
    Commits <- {k1, k2}, Parent <- [x \in {k1, k2} |-> NoCommit], Secret <- {},
    CodeOwners <- [x \in {k1, k2, NoCommit} |-> {}], CheckNames <- {}, Labels <- {},
    Content <- [x \in {"none", "bind", "main"} |->
                  CASE x = "bind" -> ch [] x = "main" -> main [] OTHER -> {}],
    Init_ <- [commRole |-> Owned,
              chanRole |-> [x \in {ch} |-> [k \in Keys |-> "admin"]],
              owner |-> [x \in {g1, g2} |-> b],
              grants |-> {<<g1, r>>, <<g2, r>>},
              gov |-> GovDocs,
              refs |-> [x \in {r} |-> [f \in {main} |-> NoCommit]],
              visible |-> [x \in {r} |-> {}], pushed |-> {}],
    GovChoices <- {}, RoleChanges <- {}, Bans <- {<<a, b>>}, Registrations <- {},
    GrantChoices <- {}, Proposals <- {<<Agent[p], r, {<<main, C[p]>>}>> : p \in Procs},
    Reviews <- {}, Posts <- {}, ReadChoices <- {}, TagChoices <- {}, Archives <- {},
    Faults <- {},
    commRole <- IF revoked THEN [Owned EXCEPT ![b] = "none"] ELSE Owned,
    chanRole <- [x \in {ch} |-> [k \in Keys |-> "admin"]],
    owner <- [x \in {g1, g2} |-> b],
    grants <- IF clock < Expiry THEN {<<g1, r>>, <<g2, r>>} ELSE {},
    gov <- GovDocs, ver <- [d \in DocIdsHere |-> 0], sent <- {}, govlog <- {},
    refs <- [x \in {r} |-> [f \in {main} |-> C[pointer]]],
    visible <- [x \in {r} |-> {C[v] : v \in Landed1}],
    pushed <- {<<Agent[v], r, C[v]>> : v \in Landed1},
    brought <- {<<Agent[v], r, C[v]>> : v \in Landed1},
    pending <- {Push(p) : p \in {p \in Procs : pc[p] \in {"judged", "refusing"}}},
    reserved <- IF rowLock = NoProc THEN {}
                ELSE {[p |-> Push(rowLock), rec |-> Rec(rowLock, TRUE)]},
    approvals <- {}, posts <- {},
    receipts <- {Stamped(x) : x \in receipts},
    audit <- {}, act <- step

GovRefined == Gov!Spec

-----------------------------------------------------------------------------
\* The mapping onto GitRefDB, from the moment the repo is created.

Outcome(p) ==
    IF \E x \in receipts : x.p = p
    THEN [f \in {main} |-> IF (CHOOSE x \in receipts : x.p = p).ok THEN "ok" ELSE "ng"]
    ELSE NoOut
Tip == [f \in {main} |-> C[pointer]]

RefDB == INSTANCE GitRefDB WITH
    Clients <- Procs, Refs <- {main}, Oids <- {k1, k2}, NoOid <- NoCommit,
    DefaultHead <- main, Owner <- b,
    db <- IF created THEN [refs |-> Tip, head |-> main] ELSE Absent,
    call <- [p \in Procs |->
               IF p \in invoked
               THEN [cmds |-> [f \in {main} |-> [old |-> C[seen[p]], new |-> C[p]]],
                     mayPush |-> judged[p]]
               ELSE NoCall],
    out <- [p \in Procs |-> Outcome(p)],
    resp <- [p \in Procs |-> NoResp],
    credit <- IF ~created THEN NoEntry
              ELSE [actor |-> IF pointer = Init0 THEN b ELSE pointer, refs |-> Tip, head |-> main]

RefDBRefined == RefDB!Spec

\* GitGovernance's promises, of this protocol, under the mapping.
GovLands     == Gov!LandsUnderCurrentAuthority
GovRefsMove  == Gov!RefsMoveOnlyByLanding
GovReceipts  == Gov!ReceiptsMatchDecisions
GovFenced    == Gov!ReservationsFenced

\* Coverage, under the mapping: a reserved push publishes after its grant
\* expired, and a reservation is abandoned by a crash.
GovNeverPublishesAfterExpiry == Gov!NeverPublishesAfterExpiry
GovNeverAborts == Gov!NeverAborts
=============================================================================
