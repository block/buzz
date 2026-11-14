-------------------------- MODULE GitOnObjectStore --------------------------
(***************************************************************************)
(* Git on object storage: the publication protocol of buzz-relay.          *)
(*                                                                         *)
(* WHAT IS CHECKED                                                         *)
(*                                                                         *)
(*   1. Refinement.  Every behavior of this module is a behavior of        *)
(*      GitRefDB (the atomic reference database) under the mapping at the  *)
(*      end of the file.  This is linearizability of ref updates, plus:    *)
(*      a client is told "ok" for a command only if that update was        *)
(*      published or the command asked for the value the ref already had,  *)
(*      a publication changes refs only on behalf of an authorized push,   *)
(*      and a push that changes nothing publishes nothing.                 *)
(*   2. Store invariants: the fence (the pointer never names a manifest or *)
(*      pack that is not stored), reader completeness (a reader that has   *)
(*      resolved the pointer can fetch everything it needs), and closure   *)
(*      (the packs of a published manifest contain every object reachable  *)
(*      from its refs).                                                    *)
(*                                                                         *)
(* HOW IT IS CHECKED                                                       *)
(*                                                                         *)
(*   docs/spec/check.sh runs TLC on this module in several configurations. *)
(*   The protocol as specified must pass.  Each element of Faults, the two *)
(*   earlier publish decisions ("legacy" and "report"), and the "constant" *)
(*   ETag mode is a seeded design error; each must be rejected with the    *)
(*   expected violation, and the script fails if one is not.               *)
(*                                                                         *)
(* MODELING DECISIONS                                                      *)
(*                                                                         *)
(*   - A digest is modeled by the value it names.  A manifest is a record  *)
(*     and "its digest" is that record; a pack is a set of objects and     *)
(*     "its digest" is that set.  This is the assumption that SHA-256 is   *)
(*     injective on the values that occur.                                 *)
(*   - The object store offers create-only writes for immutable keys and   *)
(*     conditional overwrite for the pointer (axioms A1-A3 of the design   *)
(*     document).  Nothing is ever deleted, except under the "gc" fault.   *)
(*   - The pointer comparison.  With EtagMode = "content" and              *)
(*     ParentLinks = FALSE the conditional write succeeds exactly when the *)
(*     pointer names a manifest with the same refs, head and packs as the  *)
(*     one that was read.  This is the weakest comparison any conforming   *)
(*     store provides: a store whose ETag changes on every write, and a    *)
(*     manifest that also records its predecessor, both make the write     *)
(*     succeed in strictly fewer situations and change nothing else.       *)
(*     Every behavior of those variants is therefore a behavior of this    *)
(*     one, so the properties are checked here without a bound on the      *)
(*     number of pushes, and the variants are checked directly with a      *)
(*     bound (MaxWrites) as a cross-check of that argument.                *)
(*   - git receive-pack is modeled by its contract (assumption G1): it     *)
(*     changes a ref only by applying a submitted command whose old value  *)
(*     matches and whose new object is connected, and only after the       *)
(*     pre-receive hook accepted the command list.  It may additionally    *)
(*     refuse any command (atomic pushes, directory/file conflicts, deny   *)
(*     rules) and it may exit abnormally after applying some commands.     *)
(*   - Hydration (pointer, manifest and packs) is one atomic read.  The    *)
(*     reader process covers the non-atomic read path.                     *)
(*   - Safety only.  No liveness property is stated or checked.            *)
(***************************************************************************)
EXTENDS Naturals, FiniteSets

CONSTANTS
    \* Shared with GitRefDB.
    Clients, Refs, Oids, NoOid, DefaultHead, Absent,
    NoCall, NoOut, NoCmd, NoResp, Error, NoEntry, Owner, Manager,
    \* Object graph: ObjParents[o] is the set of objects that o refers to.
    ObjParents,
    NoManifest,     \* the "parent" of a manifest that has no predecessor
    NoRead,         \* the reader has not resolved the pointer
    CompactAt,      \* a publish compacts when its base lists this many packs
    MaxWrites,      \* bound on pointer writes, where they are counted
    \* Which background processes take part (see check.sh for the reason).
    HasSetter, HasReader,
    \* Design switches.
    StaleClients,   \* may a client name an old value other than the hydrated one?
    GitMayCrash,    \* may receive-pack exit non-zero?
    ParentLinks,    \* does a manifest record its predecessor?
    Decision,       \* "ground-truth" (implemented), or an earlier rule:
                    \* "legacy" or "report"
    EtagMode,       \* "content", "version", or "constant" (a broken store)
    Faults          \* seeded design errors; {} for the protocol as specified

ASSUME DefaultHead \in Refs
ASSUME NoOid \notin Oids
ASSUME ObjParents \in [Oids -> SUBSET Oids]
ASSUME StaleClients \in BOOLEAN /\ ParentLinks \in BOOLEAN /\ GitMayCrash \in BOOLEAN
ASSUME HasSetter \in BOOLEAN /\ HasReader \in BOOLEAN
ASSUME Decision \in {"ground-truth", "legacy", "report"}
ASSUME EtagMode \in {"content", "version", "constant"}
ASSUME Faults \subseteq {"no-cas-guard", "stale-base", "drop-parent-packs",
                         "cas-before-upload", "gc", "no-hook",
                         "no-connectivity", "ack-before-cas"}

VARIABLES
    \* The object store.
    pointer,   \* Absent, or [body |-> manifest, etag |-> ...]
    mstore,    \* manifests stored (create-only)
    pstore,    \* packs stored (create-only)
    writes,    \* pointer writes so far (counted only where the count is observable)
    \* One push request per client at a time.
    pc,        \* "idle", "arrived", "relay", "publish", "uploaded", "abort", "answer"
    base,      \* pointer value the workspace was hydrated from
    call,      \* [cmds, mayPush] as submitted, or NoCall
    stat,      \* per-ref status computed by git receive-pack
    staged,    \* manifest this push will publish, or NoManifest
    lin,       \* outcome fixed at the linearization point, or NoOut
    resp,      \* what the client was last told
    \* The default-branch setter.
    spc,       \* "idle" or "staged"
    sbase,     \* pointer value the setter read
    starget,   \* branch it was asked to make the default
    \* A reader (clone or fetch).
    rd,        \* NoRead, or the manifest it resolved from the pointer
    \* History.
    credit     \* who the published state is attributed to
vars == <<pointer, mstore, pstore, writes, pc, base, call, stat,
          staged, lin, resp, spc, sbase, starget, rd, credit>>

-----------------------------------------------------------------------------
\* Values

OidOrNone  == Oids \cup {NoOid}
EmptyRefs  == [r \in Refs |-> NoOid]
\* A command may name the same value as old and new.  git accepts such a
\* command, reports "ok" for it, and changes nothing.
Command    == [old : OidOrNone, new : OidOrNone]
CommandSet == [Refs -> Command \cup {NoCmd}]
NoReport   == [r \in Refs |-> "none"]
Packs      == SUBSET Oids

\* The command sets a client may submit against hydrated refs P.  A client
\* computes its commands from an advertisement fetched by an earlier request,
\* so the old values it names can be out of date.  receive-pack refuses such
\* a command, which makes it indistinguishable from any other refused
\* command; StaleClients = FALSE leaves them out to keep the two-client
\* configuration small, and a one-client configuration checks them.
ClientCommands(P) ==
    IF StaleClients THEN CommandSet
    ELSE {cs \in CommandSet : \A r \in Refs : cs[r] # NoCmd => cs[r].old = P[r]}

Fault(f) == f \in Faults

\* Objects reachable from a set of objects.
RECURSIVE Closure(_)
Closure(S) ==
    LET T == S \cup UNION {ObjParents[o] : o \in S}
    IN IF T = S THEN S ELSE Closure(T)

Tips(refs)      == {refs[r] : r \in Refs} \ {NoOid}
Reachable(refs) == Closure(Tips(refs))
ObjectsOf(ps)   == UNION ps
PackOf(objs)    == IF objs = {} THEN {} ELSE {objs}

EmptyManifest ==
    [refs |-> EmptyRefs, head |-> DefaultHead, packs |-> {}, parent |-> NoManifest]

\* The ETag the store assigns to a pointer write.  "content" is what S3 and
\* MinIO do for a simple PUT (the ETag is a function of the body); "version"
\* is a store that issues a new ETag on every write; "constant" is a store
\* whose ETag does not distinguish bodies, which violates axiom A3.
EtagOf(body, n) ==
    CASE EtagMode = "content"  -> body
      [] EtagMode = "version"  -> n
      [] EtagMode = "constant" -> "etag"

\* Pointer writes are counted when the count is observable in the state: as
\* a version ETag, or as the depth of the predecessor chain.  In those modes
\* the state space is infinite and MaxWrites bounds the exploration.
Counted    == EtagMode = "version" \/ ParentLinks
NextWrites == IF Counted THEN writes + 1 ELSE writes
Bounded    == writes <= MaxWrites

Successor(b) == IF b = Absent \/ ~ParentLinks THEN NoManifest ELSE b.body

BaseRefs(b)  == IF b = Absent THEN EmptyRefs ELSE b.body.refs
BaseHead(b)  == IF b = Absent THEN DefaultHead ELSE b.body.head
BasePacks(b) == IF b = Absent THEN {} ELSE b.body.packs

\* cas_publish.rs, resolve_published_head: keep HEAD on its branch while that
\* branch is published; otherwise prefer the conventional default, then any
\* published branch; with no branch at all, leave HEAD where it was.
PublishedHead(refs, h) ==
    IF refs[h] # NoOid THEN h
    ELSE IF refs[DefaultHead] # NoOid THEN DefaultHead
    ELSE IF \E r \in Refs : refs[r] # NoOid
         THEN CHOOSE r \in Refs : refs[r] # NoOid
         ELSE h

\* The conditional pointer write (store.rs, put_pointer).  A request based on
\* an absent pointer uses If-None-Match: *; any other uses If-Match: <etag>.
CasWouldWin(b) ==
    \/ Fault("no-cas-guard")
    \/ b = Absent /\ pointer = Absent
    \/ b # Absent /\ pointer # Absent /\ pointer.etag = b.etag

Publication(actor, m) == [actor |-> actor, refs |-> m.refs, head |-> m.head]

-----------------------------------------------------------------------------
\* git receive-pack and the publish decision

\* Commands receive-pack is willing to apply, given the hydrated refs P and
\* the objects available (hydrated packs plus the client's pack).
ValidCmds(cmds, P, objs) ==
    {r \in Refs :
        /\ cmds[r] # NoCmd
        /\ cmds[r].old = P[r]
        /\ \/ cmds[r].new = NoOid
           \/ Fault("no-connectivity")
           \/ Closure({cmds[r].new}) \subseteq objs}

\* The pre-receive hook is run only when there is at least one command, and
\* then decides for the whole command list.
HookPermits(cmds, may) ==
    \/ Fault("no-hook")
    \/ may
    \/ \A r \in Refs : cmds[r] = NoCmd

StatusOf(cmds, applied) ==
    [r \in Refs |-> IF r \in applied THEN "ok"
                    ELSE IF cmds[r] # NoCmd THEN "ng"
                    ELSE "none"]

\* The two rules transport.rs used earlier.  Both read the report-status
\* stream, and a client that did not negotiate report-status produces no such
\* stream, so neither an "ok" nor an "ng" line is seen for it.
\*   legacy: publish when the process exited zero and the stream contains no
\*   "ng" line.
\*   report: publish when the process exited zero and the stream contains an
\*   "ok" line or no "ng" line.
SeenOk(wantsReport, st) == wantsReport /\ \E r \in Refs : st[r] = "ok"
SeenNg(wantsReport, st) == wantsReport /\ \E r \in Refs : st[r] = "ng"

LegacyOk(exitOk, wantsReport, st) == exitOk /\ ~SeenNg(wantsReport, st)

ReportOk(exitOk, wantsReport, st) ==
    exitOk /\ (SeenOk(wantsReport, st) \/ ~SeenNg(wantsReport, st))

\* Only the earlier rules read whether report-status was negotiated.
ReportChoices == IF Decision = "ground-truth" THEN {TRUE} ELSE BOOLEAN

ExitChoices == IF GitMayCrash THEN BOOLEAN ELSE {TRUE}

\* The disposition of a push after receive-pack returns.
\*   ground-truth: compare the workspace refs with the refs it was hydrated
\*   from.  Unchanged: nothing to publish.  Changed and exited zero: publish.
\*   Changed and exited non-zero: abort.
Disposition(exitOk, wantsReport, st, changed) ==
    CASE Decision = "legacy" ->
           IF LegacyOk(exitOk, wantsReport, st) THEN "publish" ELSE "relay"
      [] Decision = "report" ->
           IF ReportOk(exitOk, wantsReport, st) THEN "publish" ELSE "relay"
      [] OTHER ->
           IF ~changed THEN "relay"
           ELSE IF exitOk THEN "publish" ELSE "abort"

\* cas_publish.rs: the manifest a push stages.  The delta pack holds the
\* objects reachable from the new refs and not from the hydrated refs; a
\* base that already lists CompactAt packs is replaced by one pack holding
\* everything reachable.
StagedManifest(b, W, objs) ==
    LET P      == BaseRefs(b)
        delta  == (Reachable(W) \ Reachable(P)) \cap objs
        grown  == (IF Fault("drop-parent-packs") THEN {} ELSE BasePacks(b))
                    \cup PackOf(delta)
        packs  == IF Cardinality(BasePacks(b)) >= CompactAt
                  THEN PackOf(Reachable(W) \cap objs)
                  ELSE grown
    IN [refs   |-> W,
        head   |-> PublishedHead(W, BaseHead(b)),
        packs  |-> packs,
        parent |-> Successor(b)]

-----------------------------------------------------------------------------
\* Push

\* A request arrives with its commands.  The commands are computed by the
\* client from an earlier advertisement (see ClientCommands).
Arrive(c) ==
    /\ pc[c] = "idle"
    /\ \E cmds \in ClientCommands(BaseRefs(pointer)), may \in BOOLEAN :
         call' = [call EXCEPT ![c] = [cmds |-> cmds, mayPush |-> may]]
    /\ pc'   = [pc EXCEPT ![c] = "arrived"]
    /\ resp' = [resp EXCEPT ![c] = NoResp]
    /\ UNCHANGED <<pointer, mstore, pstore, writes, base, stat, staged, lin,
                   spc, sbase, starget, rd, credit>>

\* The relay reads the pointer once, hydrates a workspace from it, and runs
\* receive-pack.  All of this is local to the request, so it is one step; the
\* request's fate (relay / publish / abort) is then fixed.
\*
\* A request that will publish nothing takes effect here, at its read of the
\* pointer: every "ok" git reports for it belongs to a command that asked for
\* the value the ref already had, and that was true of the published state at
\* this step.  It need not be true any later.
Begin(c) ==
    /\ pc[c] = "arrived"
    /\ pointer # Absent => pointer.body \in mstore /\ pointer.body.packs \subseteq pstore
    /\ call[c].cmds \in ClientCommands(BaseRefs(pointer))
    /\ \E wantsReport \in ReportChoices, clientPack \in Packs, exitOk \in ExitChoices :
         LET cmds  == call[c].cmds
             P     == BaseRefs(pointer)
             objs  == ObjectsOf(BasePacks(pointer)) \cup clientPack
             valid == IF HookPermits(cmds, call[c].mayPush)
                      THEN ValidCmds(cmds, P, objs) ELSE {}
         IN \E applied \in SUBSET valid :
              LET W    == [r \in Refs |-> IF r \in applied THEN cmds[r].new ELSE P[r]]
                  st   == StatusOf(cmds, applied)
                  disp == Disposition(exitOk, wantsReport, st, W # P)
              IN /\ pc'     = [pc EXCEPT ![c] = disp]
                 /\ stat'   = [stat EXCEPT ![c] = IF disp = "abort" THEN NoReport ELSE st]
                 /\ base'   = [base EXCEPT ![c] = IF disp = "publish" THEN pointer ELSE Absent]
                 /\ staged' = [staged EXCEPT ![c] =
                                 IF disp = "publish"
                                 THEN StagedManifest(pointer, W, objs)
                                 ELSE NoManifest]
                 /\ lin'    = IF disp = "relay" THEN [lin EXCEPT ![c] = st] ELSE lin
    /\ UNCHANGED <<pointer, mstore, pstore, writes, call, resp,
                   spc, sbase, starget, rd, credit>>

\* The relay answers from git's output and publishes nothing.
Skip(c) ==
    /\ pc[c] = "relay"
    /\ pc'  = [pc EXCEPT ![c] = "answer"]
    /\ UNCHANGED <<pointer, mstore, pstore, writes, base, call, stat,
                   staged, lin, resp, spc, sbase, starget, rd, credit>>

\* Steps 4-6 of the write path: packs first, then the manifest.
Upload(c) ==
    /\ pc[c] = "publish"
    /\ ~Fault("cas-before-upload")
    /\ pstore' = pstore \cup staged[c].packs
    /\ mstore' = mstore \cup {staged[c]}
    /\ pc' = [pc EXCEPT ![c] = "uploaded"]
    /\ UNCHANGED <<pointer, writes, base, call, stat, staged, lin, resp,
                   spc, sbase, starget, rd, credit>>

\* Step 7: the conditional pointer write.  Winning it is the linearization
\* point of the push.  Losing it changes nothing and is covered by Fail.
CasWin(c) ==
    /\ pc[c] = IF Fault("cas-before-upload") THEN "publish" ELSE "uploaded"
    /\ CasWouldWin(base[c])
    /\ pointer' = [body |-> staged[c], etag |-> EtagOf(staged[c], NextWrites)]
    /\ writes'  = NextWrites
    /\ credit'  = Publication(c, staged[c])
    /\ lin'     = [lin EXCEPT ![c] = stat[c]]
    /\ pc'      = [pc EXCEPT ![c] = "answer"]
    /\ UNCHANGED <<mstore, pstore, base, call, stat, staged, resp,
                   spc, sbase, starget, rd>>

\* Fault "cas-before-upload": the upload happens after the pointer moved.
LateUpload(c) ==
    /\ Fault("cas-before-upload")
    /\ pc[c] = "answer"
    /\ staged[c] # NoManifest
    /\ pstore' = pstore \cup staged[c].packs
    /\ mstore' = mstore \cup {staged[c]}
    /\ UNCHANGED <<pointer, writes, pc, base, call, stat, staged, lin,
                   resp, spc, sbase, starget, rd, credit>>

\* Fault "stale-base": the pointer is read a second time for the CAS
\* precondition, while the staged manifest was derived from the first read.
Reread(c) ==
    /\ Fault("stale-base")
    /\ pc[c] = "uploaded"
    /\ pointer # Absent
    /\ base' = [base EXCEPT ![c] = pointer]
    /\ UNCHANGED <<pointer, mstore, pstore, writes, pc, call, stat,
                   staged, lin, resp, spc, sbase, starget, rd, credit>>

Reset(c) ==
    /\ pc'     = [pc EXCEPT ![c] = "idle"]
    /\ call'   = [call EXCEPT ![c] = NoCall]
    /\ stat'   = [stat EXCEPT ![c] = NoReport]
    /\ base'   = [base EXCEPT ![c] = Absent]
    /\ staged' = [staged EXCEPT ![c] = NoManifest]
    /\ lin'    = [lin EXCEPT ![c] = NoOut]

\* The response.  It carries git's own status report, or no report if the
\* client did not negotiate one.  Whether it did is not recorded: only the
\* earlier rules read it, and both answers are allowed here in every case.
Respond(c) ==
    /\ \/ pc[c] = "answer"
       \/ Fault("ack-before-cas") /\ pc[c] = "uploaded"
    /\ \E answer \in {stat[c], NoReport} : resp' = [resp EXCEPT ![c] = answer]
    /\ Reset(c)
    /\ UNCHANGED <<pointer, mstore, pstore, writes, spc, sbase, starget, rd, credit>>

\* Any failure: a lost CAS race (409), an aborted publish (500), a crash, a
\* timeout, or a response lost after the pointer moved.  The client learns
\* nothing about the outcome.
Fail(c) ==
    /\ pc[c] # "idle"
    /\ resp' = [resp EXCEPT ![c] = Error]
    /\ Reset(c)
    /\ UNCHANGED <<pointer, mstore, pstore, writes, spc, sbase, starget, rd, credit>>

-----------------------------------------------------------------------------
\* Repository creation, the default-branch setter, readers, and deletion

\* handlers/side_effects.rs, seed_manifest_pointer: on announcement, create
\* the pointer for an empty repository with If-None-Match: *.
Seed ==
    /\ pointer = Absent
    /\ mstore'  = mstore \cup {EmptyManifest}
    /\ pointer' = [body |-> EmptyManifest, etag |-> EtagOf(EmptyManifest, NextWrites)]
    /\ writes'  = NextWrites
    /\ credit'  = Publication(Owner, EmptyManifest)
    /\ UNCHANGED <<pstore, pc, base, call, stat, staged, lin, resp,
                   spc, sbase, starget, rd>>

\* settings.rs: the manifest the setter writes.  A request naming the branch
\* that is already the default writes the manifest it read.
SetManifestOf(b, h) ==
    IF b.body.head = h THEN b.body
    ELSE [b.body EXCEPT !.head = h, !.parent = Successor(b)]

\* Read the pointer, validate the requested branch against the manifest that
\* was read, and store the successor manifest.
SetBegin ==
    /\ HasSetter
    /\ spc = "idle"
    /\ pointer # Absent
    /\ \E h \in Refs :
         /\ pointer.body.refs[h] # NoOid
         /\ starget' = h
         /\ mstore' = mstore \cup {SetManifestOf(pointer, h)}
    /\ sbase' = pointer
    /\ spc' = "staged"
    /\ UNCHANGED <<pointer, pstore, writes, pc, base, call, stat, staged,
                   lin, resp, rd, credit>>

SetReset ==
    /\ spc' = "idle"
    /\ sbase' = Absent
    /\ starget' = DefaultHead

\* The setter always performs the conditional write, also when the branch is
\* already the default; that case rewrites the same body.
SetCas ==
    /\ spc = "staged"
    /\ CasWouldWin(sbase)
    /\ LET m == SetManifestOf(sbase, starget)
       IN /\ pointer' = [body |-> m, etag |-> EtagOf(m, NextWrites)]
          /\ credit'  = IF m = sbase.body THEN credit ELSE Publication(Manager, m)
    /\ writes' = NextWrites
    /\ SetReset
    /\ UNCHANGED <<mstore, pstore, pc, base, call, stat, staged, lin,
                   resp, rd>>

SetFail ==
    /\ spc = "staged"
    /\ SetReset
    /\ UNCHANGED <<pointer, mstore, pstore, writes, pc, base, call, stat,
                   staged, lin, resp, rd, credit>>

\* A reader resolves the pointer.  It fetches the manifest and packs later,
\* at any time; ReaderComplete says that fetch cannot fail.
Read ==
    /\ HasReader
    /\ pointer # Absent
    /\ rd' = pointer.body
    /\ UNCHANGED <<pointer, mstore, pstore, writes, pc, base, call, stat,
                   staged, lin, resp, spc, sbase, starget, credit>>

\* Fault "gc": delete a pack that the current manifest does not list.
Collect ==
    /\ Fault("gc")
    /\ pointer # Absent
    /\ \E p \in pstore \ pointer.body.packs : pstore' = pstore \ {p}
    /\ UNCHANGED <<pointer, mstore, writes, pc, base, call, stat, staged,
                   lin, resp, spc, sbase, starget, rd, credit>>

-----------------------------------------------------------------------------
Init ==
    /\ pointer = Absent
    /\ mstore = {}
    /\ pstore = {}
    /\ writes = 0
    /\ pc     = [c \in Clients |-> "idle"]
    /\ base   = [c \in Clients |-> Absent]
    /\ call   = [c \in Clients |-> NoCall]
    /\ stat   = [c \in Clients |-> NoReport]
    /\ staged = [c \in Clients |-> NoManifest]
    /\ lin    = [c \in Clients |-> NoOut]
    /\ resp   = [c \in Clients |-> NoResp]
    /\ spc = "idle"
    /\ sbase = Absent
    /\ starget = DefaultHead
    /\ rd = NoRead
    /\ credit = NoEntry

Next ==
    \/ Seed \/ SetBegin \/ SetCas \/ SetFail \/ Read \/ Collect
    \/ \E c \in Clients :
         \/ Arrive(c) \/ Begin(c) \/ Skip(c) \/ Upload(c) \/ CasWin(c)
         \/ LateUpload(c) \/ Reread(c) \/ Respond(c) \/ Fail(c)

Spec == Init /\ [][Next]_vars

-----------------------------------------------------------------------------
\* Store invariants

Stored(m) == m \in mstore /\ m.packs \subseteq pstore

\* T1, fence: the pointer never names anything that is not stored.
Fence == pointer # Absent => Stored(pointer.body)

\* T1 for readers: what a reader resolved stays fetchable.
ReaderComplete == rd # NoRead => Stored(rd)

\* T2, closure: the packs a manifest lists contain every object reachable
\* from its refs, so a workspace built from them alone is a complete
\* repository.
Closed(m) == Reachable(m.refs) \subseteq ObjectsOf(m.packs)
PublishedClosed ==
    /\ pointer # Absent => Closed(pointer.body)
    /\ rd # NoRead => Closed(rd)

\* The published HEAD names a published branch whenever one exists.
HeadPublished ==
    pointer # Absent =>
        ((\E r \in Refs : pointer.body.refs[r] # NoOid)
            => pointer.body.refs[pointer.body.head] # NoOid)

StoreInvariants ==
    /\ Fence
    /\ ReaderComplete
    /\ PublishedClosed
    /\ HeadPublished

\* Two consequences of the refinement, stated on their own so that a
\* violation produces a short, specific counterexample.
\*
\* A request that is answered from git's output without publishing reported
\* "ok" only for commands that asked for no change.
Changes(c, r) == call[c].cmds[r].new # call[c].cmds[r].old
NoSilentDrop ==
    \A c \in Clients :
        pc[c] = "relay" => \A r \in Refs : stat[c][r] = "ok" => ~Changes(c, r)

\* A request that reaches the publish path was accepted by the push policy
\* and git applied at least one command that changes a ref.
PublishIsAuthorizedChange ==
    \A c \in Clients :
        pc[c] \in {"publish", "uploaded"} =>
            /\ call[c].mayPush
            /\ \E r \in Refs : stat[c][r] = "ok" /\ Changes(c, r)

\* Not a requirement.  check.sh requires this to be VIOLATED by the protocol
\* as specified, which shows that the model reaches the case NoSilentDrop
\* makes an exception for: git reports "ok" for a command that asked for no
\* change, and the relay passes that on without publishing.
NeverRelaysOk ==
    \A c \in Clients : pc[c] = "relay" => \A r \in Refs : stat[c][r] # "ok"

\* With parent links, a pointer write that changes the manifest names the
\* manifest it replaced.  The published manifests then form a single chain,
\* which is what makes the predecessor field usable as an audit trail.
ParentIsPredecessor ==
    [][(ParentLinks /\ pointer # Absent /\ pointer'.body # pointer.body)
          => pointer'.body.parent = pointer.body]_vars

-----------------------------------------------------------------------------
\* Refinement: this protocol implements the atomic reference database.

AbsDb ==
    IF pointer = Absent THEN Absent
    ELSE [refs |-> pointer.body.refs, head |-> pointer.body.head]

Abs == INSTANCE GitRefDB WITH db <- AbsDb, out <- lin

Refines == Abs!Spec

-----------------------------------------------------------------------------
\* Names used by comments in the implementation and in migration 0002, which
\* were written against an earlier version of this module.  Each now stands
\* for the property given here.
Inv_NoFork               == Refines
Inv_RefEffectApplied     == Refines
Inv_RefDerivedFromParent == ParentIsPredecessor
Inv_Closed               == PublishedClosed
=============================================================================
