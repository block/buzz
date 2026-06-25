-------------------------- MODULE MultiTenantRelay --------------------------
(***************************************************************************)
(* Formal model of Buzz's proposed multi-tenant relay/database isolation.    *)
(*                                                                         *)
(* This is the TLA+ half of the multi-tenant relay proof.  It models N       *)
(* stateless relay workers over one shared Postgres database containing a    *)
(* community_id-keyed canonical message log, tenant-scoped control-plane     *)
(* state, and rebuildable projections.                                      *)
(*                                                                         *)
(* The master proof obligation is NOT merely "no row with the wrong          *)
(* community_id is returned."  The theorem contract is non-interference      *)
(* encoded as a label/taint invariant: every state element and every         *)
(* observation carries the community labels that influence it; no value      *)
(* labeled outside a connection's resolved community may flow into that       *)
(* connection's typed observational interface.                              *)
(*                                                                         *)
(* C1 carve-out (not modeled as a security theorem): bandwidth-limited       *)
(* physical resource timing channels such as buffer cache, autovacuum,       *)
(* planner stats, hot partition tails, and connection-pool latency.          *)
(*                                                                         *)
(* C2 channels modeled here and closed by invariants:                        *)
(*   - event-id existence oracle: write conflict checks are scoped by        *)
(*     (community, id), not global id; cross-community same-id writes do     *)
(*     not suppress each other. A_HASH covers adversarial preimage probing.  *)
(*   - constraint/error surface: the relay emits only a fixed sanitized      *)
(*     error alphabet; sanitized error observations are relay-static and      *)
(*     carry no tenant label.                                                *)
(*   - projection rebuild: rebuild touches all communities internally but    *)
(*     emits no tenant observation; tenant reads see only own projection      *)
(*     rows (or a subset/none during rebuild).                               *)
(*                                                                         *)
(* Source grounding from today's Buzz:                                      *)
(*   - migrations/0001_initial_schema.sql: events, channels,                *)
(*     channel_members, event_mentions, thread_metadata, reactions,          *)
(*     workflows, api_tokens, relay_members.                                *)
(*   - crates/buzz-db/src/event.rs: EventQuery has channel_id/channel_ids    *)
(*     but no community_id; inserts use ON CONFLICT DO NOTHING.              *)
(*   - crates/buzz-db/src/channel.rs: get_accessible_channel_ids currently   *)
(*     unions all open channels in the DB; that unscoped variant is the      *)
(*     explicit I1 mutation.                                                 *)
(*   - crates/buzz-relay/src/state.rs: process-global AppState/caches today. *)
(*                                                                         *)
(* Mutation tests to keep non-vacuous:                                      *)
(*   M1: ReadScoped uses UnscopedAccessible(actor) instead of                *)
(*       ScopedAccessible(community, actor) -> Inv_NonInterference breaks.   *)
(*   M2: WriteInsert/AuthCheck use claimedCommunity/h tag instead of         *)
(*       ChannelCommunity[channel] -> resolution-fence invariants break.     *)
(*   M3: WriteDuplicate conflict on id only (GlobalConflictRows + guard         *)
(*       conflicts # {}), not (community,id) -> cross-community suppression      *)
(*       labels a B write result with A and Inv_NonInterference breaks.          *)
(*       Confirmed red: Safety violated at depth 3 (see GlobalConflictRows).     *)
(*   M4: ReadForgotPredicateWithRLS returns candidates without RLSRows ->    *)
(*       Inv_NonInterference breaks.                                         *)
(*   M5: Projection rebuild emits an observation or projection reads ignore  *)
(*       row labels -> Inv_NonInterference breaks.                           *)
(*   M6: Error observation carries raw/high labels or a value outside the    *)
(*       sanitized alphabet -> Inv_SanitizedErrors/NI breaks.                *)
(*   M7: Direct ids lookup ignores ctx and resolves scope from the row/global *)
(*       id index -> a B-scoped observation can carry an A-labeled row.       *)
(***************************************************************************)
EXTENDS FiniteSets, Naturals, TLC

CONSTANTS
    Communities,       \* finite set of community ids
    Channels,          \* finite set of channel ids
    Actors,            \* finite set of pubkeys/actors
    Workers,           \* finite set of relay worker/process ids
    MsgIds,            \* finite set of event ids (model bound)
    AuditVals,         \* finite set of audit head values (model bound)
    CommA,             \* model value: first community in TLC config
    CommB,             \* model value: second community in TLC config
    ChanA1,            \* model value: community-A channel in TLC config
    ChanA2,            \* model value: community-A channel in TLC config
    ChanB1,            \* model value: community-B channel in TLC config
    ChanB2,            \* model value: community-B channel in TLC config
    SanitizedErrors    \* fixed WebSocket-reachable sanitized error alphabet

ObsKinds == {"ResultRows", "WriteResult", "SanitizedError", "AuditHead", "AuthVerdict"}
MaxObservations == 2
WriteResults == {"Inserted", "Duplicate", "None"}
AuthVerdicts == {"Allow", "Deny", "None"}
NoError == "NoError"
NoAudit == "NoAudit"

ChannelCommunity == [ch \in Channels |-> IF ch \in {ChanA1, ChanA2} THEN CommA ELSE CommB]

Symmetry ==
    Permutations(Actors) \cup
    Permutations(Workers) \cup
    Permutations(MsgIds) \cup
    Permutations(AuditVals)

VARIABLES
    messages,          \* set of canonical message rows (source="message")
    projections,       \* set of rebuildable projection rows (source="projection")
    memberships,       \* tenant-scoped active channel membership rows
    openChannels,      \* set of open/public channel ids
    auditHeads,        \* function: community -> current audit head
    observations,      \* typed outputs visible to tenant-scoped clients
    acceptedWrites,    \* write requests that inserted a new message row
    duplicateWrites,   \* write requests that no-op'd on scoped conflict
    queryFaults        \* fail-closed query-layer faults (e.g. no TenantContext)

vars == <<messages, projections, memberships, openChannels, auditHeads,
          observations, acceptedWrites, duplicateWrites, queryFaults>>

DataRows == [
    id        : MsgIds,
    community : Communities,
    channel   : Channels,
    author    : Actors,
    source    : {"message", "projection"}
]

MessageRows == {r \in DataRows : r.source = "message"}
ProjectionRows == {r \in DataRows : r.source = "projection"}

MembershipRows == [
    community : Communities,
    channel   : Channels,
    actor     : Actors
]

AcceptedWriteRows == [
    worker           : Workers,
    id               : MsgIds,
    community        : Communities,
    channel          : Channels,
    author           : Actors,
    claimedCommunity : Communities
]

DuplicateWriteRows == [
    worker           : Workers,
    id               : MsgIds,
    community        : Communities,
    channel          : Channels,
    author           : Actors,
    claimedCommunity : Communities
]

Observations == [
    worker    : Workers,
    actor     : Actors,
    community : Communities,          \* resolved/request TenantContext community
    channel   : Channels,             \* target channel for channel-scoped ops
    kind      : ObsKinds,
    labels    : SUBSET Communities,   \* taint labels influencing this observation
    rows      : SUBSET DataRows,       \* row/projection dependencies, if any
    error     : SanitizedErrors \cup {NoError},
    result    : WriteResults,
    verdict   : AuthVerdicts,
    audit     : AuditVals \cup {NoAudit}
]

QueryFaultRows == [
    worker    : Workers,
    actor     : Actors,
    community : Communities,
    reason    : {"missing_tenant_context"}
]

MessageRow(id, c, ch, a) ==
    [id |-> id, community |-> c, channel |-> ch, author |-> a, source |-> "message"]

ProjectionRow(m) ==
    [id |-> m.id, community |-> m.community, channel |-> m.channel,
     author |-> m.author, source |-> "projection"]

RowLabels(rows) == {r.community : r \in rows}

MessageKeys == {[community |-> m.community, id |-> m.id] : m \in messages}

ScopedConflictRows(c, id) == {m \in messages : m.community = c /\ m.id = id}
\* Intentionally-bad global conflict set for mutation M3 (the missing-
\* community_id-in-the-unique-index footgun, i.e. UNIQUE(id) instead of
\* UNIQUE(community_id,...,id)).  To run M3: in WriteDuplicate substitute
\*   conflicts == ScopedConflictRows(real, id)
\* and change the duplicate guard from  key \in MessageKeys  to  conflicts # {}
\* (a global index fires the dup branch whenever the id exists in ANY community).
\* Confirmed red: Invariant Safety violated at depth 3, with a B-scoped
\* WriteResult observation carrying labels |-> {commA} (the C2.1 existence-oracle
\* leak).  Closure is A-RLS-5 (composite index), with A_HASH as supporting axiom.
GlobalConflictRows(id) == {m \in messages : m.id = id}

DerivedProjectionRows == {ProjectionRow(m) : m \in messages}

TypeOK ==
    /\ Communities # {}
    /\ Channels # {}
    /\ Actors # {}
    /\ Workers # {}
    /\ MsgIds # {}
    /\ AuditVals # {}
    /\ ChannelCommunity \in [Channels -> Communities]
    /\ CommA \in Communities
    /\ CommB \in Communities
    /\ CommA # CommB
    /\ {ChanA1, ChanA2, ChanB1, ChanB2} \subseteq Channels
    /\ ChanA1 # ChanB1
    /\ SanitizedErrors # {}
    /\ NoError \notin SanitizedErrors
    /\ NoAudit \notin AuditVals
    /\ messages \subseteq MessageRows
    /\ projections \subseteq ProjectionRows
    /\ projections \subseteq DerivedProjectionRows
    /\ memberships \subseteq MembershipRows
    /\ openChannels \subseteq Channels
    /\ auditHeads \in [Communities -> AuditVals]
    /\ observations \subseteq Observations
    /\ acceptedWrites \subseteq AcceptedWriteRows
    /\ duplicateWrites \subseteq DuplicateWriteRows
    /\ queryFaults \subseteq QueryFaultRows
    \* Tenant-scoped control plane: a membership row's community agrees with
    \* the server-owned channel -> community mapping.
    /\ \A m \in memberships : m.community = ChannelCommunity[m.channel]
    \* Message/projection rows are stamped with the server-owned channel mapping.
    /\ \A m \in messages : m.community = ChannelCommunity[m.channel]
    /\ \A p \in projections : p.community = ChannelCommunity[p.channel]

Init ==
    /\ messages = {}
    /\ projections = {}
    /\ memberships = {}
    /\ openChannels = {}
    /\ auditHeads \in [Communities -> AuditVals]
    /\ observations = {}
    /\ acceptedWrites = {}
    /\ duplicateWrites = {}
    /\ queryFaults = {}

ScopedAccessible(community, actor) ==
    {ch \in Channels :
        /\ ChannelCommunity[ch] = community
        /\ (ch \in openChannels \/
            [community |-> community, channel |-> ch, actor |-> actor]
                \in memberships)}

\* Intentionally-bad operator matching today's shared-DB landmine: open channels
\* are global, not scoped by TenantContext.  The correct spec does not call this;
\* substitute it into ReadScoped/ReadProjectionRows for mutation M1.
UnscopedAccessible(actor) ==
    {ch \in Channels :
        ch \in openChannels \/
        \E c \in Communities :
            [community |-> c, channel |-> ch, actor |-> actor] \in memberships}

VisibleMessageRows(community, actor) ==
    {m \in messages :
        /\ m.community = community
        /\ m.channel \in ScopedAccessible(community, actor)}

VisibleProjectionRows(community, actor) ==
    {p \in projections :
        /\ p.community = community
        /\ p.channel \in ScopedAccessible(community, actor)}

VisibleDirectIdRows(community, actor, id) ==
    {m \in messages :
        /\ m.id = id
        /\ m.community = community
        /\ m.channel \in ScopedAccessible(community, actor)}

\* Intentionally-bad direct lookup mutation: answer by global id first and trust
\* the row's own community as scope.  Substitute this into ReadByIdRows for M7.
UnscopedDirectIdRows(actor, id) ==
    {m \in messages :
        /\ m.id = id
        /\ m.channel \in UnscopedAccessible(actor)}

RLSRows(community, rows) == {r \in rows : r.community = community}

WriteInsert(w) ==
    /\ Cardinality(observations) < MaxObservations
    /\ \E id \in MsgIds, ch \in Channels, a \in Actors, claimed \in Communities :
        LET real == ChannelCommunity[ch]
            key == [community |-> real, id |-> id]
            row == MessageRow(id, real, ch, a)
            obs == [worker |-> w, actor |-> a, community |-> real, channel |-> ch,
                    kind |-> "WriteResult", labels |-> {real}, rows |-> {row},
                    error |-> NoError, result |-> "Inserted", verdict |-> "None", audit |-> NoAudit]
            wr  == [worker |-> w, id |-> id, community |-> real,
                    channel |-> ch, author |-> a, claimedCommunity |-> claimed]
        IN
            /\ key \notin MessageKeys
            /\ messages' = messages \cup {row}
            /\ observations' = observations \cup {obs}
            /\ acceptedWrites' = acceptedWrites \cup {wr}
            /\ UNCHANGED <<projections, memberships, openChannels, auditHeads,
                          duplicateWrites, queryFaults>>

WriteDuplicate(w) ==
    /\ Cardinality(observations) < MaxObservations
    /\ \E id \in MsgIds, ch \in Channels, a \in Actors, claimed \in Communities :
        LET real == ChannelCommunity[ch]
            key == [community |-> real, id |-> id]
            conflicts == ScopedConflictRows(real, id)
            obs == [worker |-> w, actor |-> a, community |-> real, channel |-> ch,
                    kind |-> "WriteResult", labels |-> RowLabels(conflicts), rows |-> conflicts,
                    error |-> NoError, result |-> "Duplicate", verdict |-> "None", audit |-> NoAudit]
            wr  == [worker |-> w, id |-> id, community |-> real,
                    channel |-> ch, author |-> a, claimedCommunity |-> claimed]
        IN
            /\ key \in MessageKeys
            /\ messages' = messages
            /\ observations' = observations \cup {obs}
            /\ duplicateWrites' = duplicateWrites \cup {wr}
            /\ UNCHANGED <<projections, memberships, openChannels, auditHeads,
                          acceptedWrites, queryFaults>>

ReadMessageRows(w) ==
    /\ Cardinality(observations) < MaxObservations
    /\ \E c \in Communities, a \in Actors, ch \in Channels :
        LET rows == VisibleMessageRows(c, a)
            obs == [worker |-> w, actor |-> a, community |-> c, channel |-> ch,
                    kind |-> "ResultRows", labels |-> RowLabels(rows), rows |-> rows,
                    error |-> NoError, result |-> "None", verdict |-> "None", audit |-> NoAudit]
        IN
            /\ observations' = observations \cup {obs}
            /\ UNCHANGED <<messages, projections, memberships, openChannels,
                          auditHeads, acceptedWrites, duplicateWrites, queryFaults>>

ReadProjectionRows(w) ==
    /\ Cardinality(observations) < MaxObservations
    /\ \E c \in Communities, a \in Actors, ch \in Channels :
        \E rows \in SUBSET VisibleProjectionRows(c, a) :
            LET obs == [worker |-> w, actor |-> a, community |-> c, channel |-> ch,
                        kind |-> "ResultRows", labels |-> RowLabels(rows), rows |-> rows,
                        error |-> NoError, result |-> "None", verdict |-> "None", audit |-> NoAudit]
            IN
                /\ observations' = observations \cup {obs}
                /\ UNCHANGED <<messages, projections, memberships, openChannels,
                              auditHeads, acceptedWrites, duplicateWrites, queryFaults>>

ReadByIdRows(w) ==
    /\ Cardinality(observations) < MaxObservations
    /\ \E c \in Communities, a \in Actors, ch \in Channels, id \in MsgIds :
        LET rows == VisibleDirectIdRows(c, a, id)
            obs == [worker |-> w, actor |-> a, community |-> c, channel |-> ch,
                    kind |-> "ResultRows", labels |-> RowLabels(rows), rows |-> rows,
                    error |-> NoError, result |-> "None", verdict |-> "None", audit |-> NoAudit]
        IN
            /\ observations' = observations \cup {obs}
            /\ UNCHANGED <<messages, projections, memberships, openChannels,
                          auditHeads, acceptedWrites, duplicateWrites, queryFaults>>

\* Explicit community predicate was accidentally omitted, but the transaction is
\* inside TenantContext and Postgres RLS applies the community fence.
ReadForgotPredicateWithRLS(w) ==
    /\ Cardinality(observations) < MaxObservations
    /\ \E c \in Communities, a \in Actors, ch \in Channels :
        LET candidates == {m \in messages : m.channel \in ScopedAccessible(c, a)}
            rows       == RLSRows(c, candidates)
            obs        == [worker |-> w, actor |-> a, community |-> c, channel |-> ch,
                           kind |-> "ResultRows", labels |-> RowLabels(rows), rows |-> rows,
                           error |-> NoError, result |-> "None", verdict |-> "None", audit |-> NoAudit]
        IN
            /\ observations' = observations \cup {obs}
            /\ UNCHANGED <<messages, projections, memberships, openChannels,
                          auditHeads, acceptedWrites, duplicateWrites, queryFaults>>

\* If the query does not establish TenantContext at all, RLS must fail closed.
ReadNoTenantContext(w) ==
    /\ Cardinality(observations) < MaxObservations
    /\ \E c \in Communities, a \in Actors, ch \in Channels :
        LET obs   == [worker |-> w, actor |-> a, community |-> c, channel |-> ch,
                      kind |-> "ResultRows", labels |-> {}, rows |-> {},
                      error |-> NoError, result |-> "None", verdict |-> "None", audit |-> NoAudit]
            fault == [worker |-> w, actor |-> a, community |-> c,
                      reason |-> "missing_tenant_context"]
        IN
            /\ observations' = observations \cup {obs}
            /\ queryFaults' = queryFaults \cup {fault}
            /\ UNCHANGED <<messages, projections, memberships, openChannels,
                          auditHeads, acceptedWrites, duplicateWrites>>

SanitizedError(w) ==
    /\ Cardinality(observations) < MaxObservations
    /\ \E c \in Communities, a \in Actors, ch \in Channels, e \in SanitizedErrors :
        LET obs == [worker |-> w, actor |-> a, community |-> c, channel |-> ch,
                    kind |-> "SanitizedError", labels |-> {}, rows |-> {},
                    error |-> e, result |-> "None", verdict |-> "None", audit |-> NoAudit]
        IN
            /\ observations' = observations \cup {obs}
            /\ UNCHANGED <<messages, projections, memberships, openChannels,
                          auditHeads, acceptedWrites, duplicateWrites, queryFaults>>

AuthCheck(w) ==
    /\ Cardinality(observations) < MaxObservations
    /\ \E ch \in Channels, a \in Actors, claimed \in Communities :
        LET real == ChannelCommunity[ch]
            allowed == ch \in ScopedAccessible(real, a)
            verdict == IF allowed THEN "Allow" ELSE "Deny"
            obs == [worker |-> w, actor |-> a, community |-> real, channel |-> ch,
                    kind |-> "AuthVerdict", labels |-> {real}, rows |-> {},
                    error |-> NoError, result |-> "None", verdict |-> verdict, audit |-> NoAudit]
        IN
            /\ observations' = observations \cup {obs}
            /\ UNCHANGED <<messages, projections, memberships, openChannels,
                          auditHeads, acceptedWrites, duplicateWrites, queryFaults>>

AppendAudit(w) ==
    \E c \in Communities, newHead \in AuditVals :
        /\ auditHeads' = [auditHeads EXCEPT ![c] = newHead]
        /\ UNCHANGED <<messages, projections, memberships, openChannels,
                      observations, acceptedWrites, duplicateWrites, queryFaults>>

ObserveAuditHead(w) ==
    /\ Cardinality(observations) < MaxObservations
    /\ \E c \in Communities, a \in Actors, ch \in Channels :
        LET obs == [worker |-> w, actor |-> a, community |-> c, channel |-> ch,
                    kind |-> "AuditHead", labels |-> {c}, rows |-> {},
                    error |-> NoError, result |-> "None", verdict |-> "None", audit |-> auditHeads[c]]
        IN
            /\ observations' = observations \cup {obs}
            /\ UNCHANGED <<messages, projections, memberships, openChannels,
                          auditHeads, acceptedWrites, duplicateWrites, queryFaults>>

\* Projection rebuild is privileged internal work. It may touch all communities
\* and may leave projections temporarily partial, but it emits no observation.
RebuildProjections(w) ==
    \E rebuilt \in SUBSET DerivedProjectionRows :
        /\ projections' = rebuilt
        /\ UNCHANGED <<messages, memberships, openChannels, auditHeads,
                      observations, acceptedWrites, duplicateWrites, queryFaults>>

AddMembership(w) ==
    \E ch \in Channels, a \in Actors :
        LET c == ChannelCommunity[ch]
            row == [community |-> c, channel |-> ch, actor |-> a]
        IN
            /\ memberships' = memberships \cup {row}
            /\ UNCHANGED <<messages, projections, openChannels, auditHeads,
                          observations, acceptedWrites, duplicateWrites, queryFaults>>

RemoveMembership(w) ==
    \E ch \in Channels, a \in Actors :
        LET c == ChannelCommunity[ch]
            row == [community |-> c, channel |-> ch, actor |-> a]
        IN
            /\ memberships' = memberships \ {row}
            /\ UNCHANGED <<messages, projections, openChannels, auditHeads,
                          observations, acceptedWrites, duplicateWrites, queryFaults>>

OpenChannel(w) ==
    \E ch \in Channels :
        /\ openChannels' = openChannels \cup {ch}
        /\ UNCHANGED <<messages, projections, memberships, auditHeads,
                      observations, acceptedWrites, duplicateWrites, queryFaults>>

CloseChannel(w) ==
    \E ch \in Channels :
        /\ openChannels' = openChannels \ {ch}
        /\ UNCHANGED <<messages, projections, memberships, auditHeads,
                      observations, acceptedWrites, duplicateWrites, queryFaults>>

Next ==
    \E w \in Workers :
        \/ WriteInsert(w)
        \/ WriteDuplicate(w)
        \/ ReadMessageRows(w)
        \/ ReadProjectionRows(w)
        \/ ReadByIdRows(w)
        \/ ReadForgotPredicateWithRLS(w)
        \/ ReadNoTenantContext(w)
        \/ SanitizedError(w)
        \/ AuthCheck(w)
        \/ AppendAudit(w)
        \/ ObserveAuditHead(w)
        \/ RebuildProjections(w)
        \/ AddMembership(w)
        \/ RemoveMembership(w)
        \/ OpenChannel(w)
        \/ CloseChannel(w)

BoundedObservations == Cardinality(observations) <= MaxObservations

Spec == Init /\ [][Next]_vars

------------------------------------------------------------------------------
\* SAFETY PROPERTIES

\* NI (master): no observation scoped to community C may be influenced by a row,
\* projection, audit head, auth decision, write-conflict source, or error source
\* labeled outside C. This is the single-run label/taint encoding of the
\* two-execution non-interference theorem.
Inv_NonInterference ==
    \A o \in observations : o.labels \subseteq {o.community}

\* Label propagation: observation labels are not arbitrary annotations; they are
\* derived from the dependencies each observation can reveal.
Inv_LabelPropagation ==
    \A o \in observations :
        /\ (o.kind \in {"ResultRows", "WriteResult"} => o.labels = RowLabels(o.rows))
        /\ (o.kind = "SanitizedError" => o.labels = {} /\ o.error \in SanitizedErrors)
        /\ (o.kind = "AuditHead" => o.labels = {o.community} /\ o.audit \in AuditVals)
        /\ (o.kind = "AuthVerdict" => o.labels = {ChannelCommunity[o.channel]} /\ o.community = ChannelCommunity[o.channel])

\* I1 read confinement follows from NI + label propagation, but is kept as a
\* legible mutation target for the current get_accessible_channel_ids landmine.
Inv_ReadConfinement ==
    \A o \in observations :
        o.kind = "ResultRows" => \A r \in o.rows : r.community = o.community

\* I2 resolution fence: persisted messages and write/auth observations are
\* labeled by the server-owned channel->community mapping, not by a client h tag.
Inv_ResolutionFence ==
    /\ \A m \in messages : m.community = ChannelCommunity[m.channel]
    /\ \A w \in acceptedWrites : w.community = ChannelCommunity[w.channel]
    /\ \A w \in duplicateWrites : w.community = ChannelCommunity[w.channel]
    /\ \A o \in observations :
        o.kind \in {"WriteResult", "AuthVerdict"} => o.community = ChannelCommunity[o.channel]

\* I3a append persistence: every accepted append remains present in the shared log.
Inv_AcceptedWritesPersist ==
    \A w \in acceptedWrites : MessageRow(w.id, w.community, w.channel, w.author) \in messages

\* I3b scoped idempotence: ids are unique within a community, not globally.  This
\* permits two different communities to store the same content hash while avoiding
\* the event-id existence oracle as a cross-tenant write-conflict channel.
Inv_MessageKeyUnique ==
    \A m1, m2 \in messages :
        (m1.community = m2.community /\ m1.id = m2.id) => (m1 = m2)

\* I4 fail-closed backstop: missing TenantContext serves no rows. Dropped SQL
\* predicates inside a valid TenantContext are covered by RLSRows and NI.
Inv_NoTenantContextFailsClosed ==
    \A o \in observations :
        (o.kind = "ResultRows" /\ o.labels = {}) => o.rows = {}

\* Projection rows are derived-only and inherit the source event label.
Inv_ProjectionDerived == projections \subseteq DerivedProjectionRows

\* Sanitized error alphabet is the only client-visible error surface in scope.
Inv_SanitizedErrors ==
    \A o \in observations :
        o.kind = "SanitizedError" => o.error \in SanitizedErrors

Safety ==
    /\ TypeOK
    /\ Inv_NonInterference
    /\ Inv_LabelPropagation
    /\ Inv_ReadConfinement
    /\ Inv_ResolutionFence
    /\ Inv_AcceptedWritesPersist
    /\ Inv_MessageKeyUnique
    /\ Inv_NoTenantContextFailsClosed
    /\ Inv_ProjectionDerived
    /\ Inv_SanitizedErrors
=============================================================================
