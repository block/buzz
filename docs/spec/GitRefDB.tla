------------------------------ MODULE GitRefDB ------------------------------
(***************************************************************************)
(* The atomic reference database: what a Git host is supposed to be.       *)
(*                                                                         *)
(* This module is the SPECIFICATION that docs/spec/GitOnObjectStore.tla is *)
(* checked against.  It contains no object store, no packs, no ETags and   *)
(* no concurrency control.  Every operation takes effect in one step.      *)
(* The protocol is correct if every one of its behaviors is also a         *)
(* behavior of this module under the refinement mapping given there; that  *)
(* statement is linearizability of ref updates, together with the          *)
(* guarantee that a client is never told an update succeeded when it was   *)
(* not applied.                                                            *)
(*                                                                         *)
(* A push is modeled as an invocation (the client submits ref-update       *)
(* commands), one atomic linearization step (the commands are evaluated    *)
(* against the current state and some of them are applied), and a response *)
(* (the client learns the per-ref outcome, or learns nothing).             *)
(*                                                                         *)
(* What this module permits, deliberately:                                 *)
(*   - Any command may be rejected.  The module states safety only: an     *)
(*     applied command had the right old value and was authorized.  It     *)
(*     does not state that a valid command must be accepted.               *)
(*   - A response may be an error even after the operation took effect.    *)
(*     A lost reply cannot be distinguished from a failed request, so the  *)
(*     guarantee runs one way: reported success implies applied.           *)
(***************************************************************************)
EXTENDS Naturals

CONSTANTS
    Clients,      \* identities that push
    Refs,         \* ref names
    Oids,         \* object ids a ref may hold
    NoOid,        \* "this ref does not exist"
    DefaultHead,  \* symbolic HEAD of a newly created repository
    Absent,       \* the repository does not exist
    NoCall, NoOut, NoCmd, NoResp, Error, NoEntry,
    Owner,        \* actor recorded for repository creation
    Manager       \* actor recorded for a default-branch change

VARIABLES
    db,    \* Absent, or [refs, head]: the published ref state
    call,  \* call[c]: the push client c has outstanding, or NoCall
    out,   \* out[c]: per-ref outcome fixed when c's push linearized, or NoOut
    resp,  \* resp[c]: what client c was last told
    credit \* who the published state is attributed to: [actor, refs, head]

vars == <<db, call, out, resp, credit>>

OidOrNone == Oids \cup {NoOid}
EmptyRefs == [r \in Refs |-> NoOid]
Command   == [old : OidOrNone, new : OidOrNone]
CommandSet == [Refs -> Command \cup {NoCmd}]
NoReport  == [r \in Refs |-> "none"]

CurRefs == IF db = Absent THEN EmptyRefs ELSE db.refs
CurHead == IF db = Absent THEN DefaultHead ELSE db.head

\* The published HEAD stays on its branch while that branch exists, and
\* otherwise moves to some existing branch (which one is left open).  It
\* dangles only when the repository has no refs at all.
HeadChoices(refs, h) ==
    IF refs[h] # NoOid THEN {h}
    ELSE IF \E r \in Refs : refs[r] # NoOid
         THEN {r \in Refs : refs[r] # NoOid}
         ELSE {h}

Entry(actor, refs, head) == [actor |-> actor, refs |-> refs, head |-> head]

Init ==
    /\ db = Absent
    /\ call = [c \in Clients |-> NoCall]
    /\ out  = [c \in Clients |-> NoOut]
    /\ resp = [c \in Clients |-> NoResp]
    /\ credit = NoEntry

\* The repository is announced.  Only this action creates an empty repository.
Create ==
    /\ db = Absent
    /\ db' = [refs |-> EmptyRefs, head |-> DefaultHead]
    /\ credit' = Entry(Owner, EmptyRefs, DefaultHead)
    /\ UNCHANGED <<call, out, resp>>

\* Client c submits a set of ref-update commands.  mayPush records whether
\* the push policy authorizes this client's command set.
Invoke(c) ==
    /\ call[c] = NoCall
    /\ \E cs \in CommandSet, may \in BOOLEAN :
         call' = [call EXCEPT ![c] = [cmds |-> cs, mayPush |-> may]]
    /\ resp' = [resp EXCEPT ![c] = NoResp]
    /\ UNCHANGED <<db, out, credit>>

\* The push takes effect.  A is the set of refs whose command is applied.
\*   - Nothing is applied without authorization.
\*   - A command is applied only against the old value it names.  A command
\*     whose new value equals its old value may be applied; it then confirms
\*     the value of the ref and changes nothing.
\*   - Every other submitted command is reported rejected and has no effect.
\*   - A push that changes no ref changes nothing, and the published state
\*     stays attributed to whoever it was attributed to before.
Linearize(c) ==
    /\ call[c] # NoCall
    /\ out[c] = NoOut
    /\ LET cs   == call[c].cmds
           Sent == {r \in Refs : cs[r] # NoCmd}
       IN \E A \in SUBSET Sent :
            /\ A # {} => call[c].mayPush
            /\ \A r \in A : cs[r].old = CurRefs[r]
            /\ LET newRefs == [r \in Refs |-> IF r \in A THEN cs[r].new ELSE CurRefs[r]]
               IN \E newHead \in HeadChoices(newRefs, CurHead) :
                  /\ out' = [out EXCEPT ![c] =
                               [r \in Refs |-> IF r \in A THEN "ok"
                                               ELSE IF r \in Sent THEN "ng"
                                               ELSE "none"]]
                  /\ IF newRefs = CurRefs
                     THEN db' = db /\ credit' = credit
                     ELSE /\ db' = [refs |-> newRefs, head |-> newHead]
                          /\ credit' = Entry(c, newRefs, newHead)
    /\ UNCHANGED <<call, resp>>

\* The client is answered.  It may be told the exact outcome, told nothing
\* (a client that did not ask for a status report), or given an error.  It is
\* told an outcome only if the push linearized, and then only the true one.
Respond(c) ==
    /\ call[c] # NoCall
    /\ \E answer \in {Error} \cup (IF out[c] = NoOut THEN {} ELSE {out[c], NoReport}) :
         resp' = [resp EXCEPT ![c] = answer]
    /\ call' = [call EXCEPT ![c] = NoCall]
    /\ out'  = [out  EXCEPT ![c] = NoOut]
    /\ UNCHANGED <<db, credit>>

\* The default branch is changed to an existing branch.
SetHead ==
    /\ db # Absent
    /\ \E h \in Refs :
         /\ db.refs[h] # NoOid
         /\ h # db.head
         /\ db' = [db EXCEPT !.head = h]
         /\ credit' = Entry(Manager, db.refs, h)
    /\ UNCHANGED <<call, out, resp>>

Next ==
    \/ Create
    \/ SetHead
    \/ \E c \in Clients : Invoke(c) \/ Linearize(c) \/ Respond(c)

Spec == Init /\ [][Next]_vars

-----------------------------------------------------------------------------
\* Consequences of the specification, checked on this module by GitRefDB.cfg.
\* They are properties of the specification, so they hold of every system
\* that refines it.

\* A push that has applied a command was authorized.
AppliedWasAuthorized ==
    \A c \in Clients :
        (out[c] # NoOut /\ \E r \in Refs : out[c][r] = "ok") => call[c].mayPush

\* The published state is attributed to the operation that produced it.
CreditMatchesState ==
    db # Absent => (credit # NoEntry /\ credit.refs = db.refs /\ credit.head = db.head)

\* A client is told an outcome only for a push that took effect, and then
\* only the true outcome.
ToldOnlyTheOutcome ==
    [][\A c \in Clients :
          (resp'[c] # resp[c] /\ resp'[c] \notin {Error, NoResp})
              => (out[c] # NoOut /\ resp'[c] \in {out[c], NoReport})]_vars

\* The published HEAD names an existing branch whenever any branch exists.
HeadNotDangling ==
    db # Absent =>
        ((\E r \in Refs : db.refs[r] # NoOid) => db.refs[db.head] # NoOid)

\* A ref changes only in a step that applies a command naming its current
\* value as the old value.  This is the "no lost update" property.
NoLostUpdate ==
    [][\A r \in Refs :
          CurRefs'[r] # CurRefs[r] =>
              \E c \in Clients :
                  /\ call[c] # NoCall
                  /\ call[c].cmds[r] # NoCmd
                  /\ call[c].cmds[r].old = CurRefs[r]
                  /\ call[c].cmds[r].new = CurRefs'[r]]_vars
=============================================================================
