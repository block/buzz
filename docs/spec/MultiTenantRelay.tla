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
(*       ChannelCommunity(channel) -> resolution-fence invariants break.     *)
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
(*   M8: WriteInsert/AuthCheck/WriteDuplicate drop the host/channel agreement   *)
(*       fence (accept/report when HostCommunity[host] # ChannelCommunity(ch)) ->*)
(*       an A-host op (insert, authz, OR duplicate-probe) on a B-channel is      *)
(*       accepted/allowed/reported -> Inv_HostBindingFence breaks (an accepted   *)
(*       write or recorded duplicate carries a host whose mapping # its stamp).  *)
(*       Confirmed red: Inv_HostBindingFence violated by a 2-state trace         *)
(*       (Init -> WriteInsert) for the insert path and a 3-state trace            *)
(*       (Init -> WriteInsert -> WriteDuplicate) for the duplicate path.  (Trace  *)
(*       length, not TLC's run-dependent "depth of complete graph search" line.)  *)
(*   M9: NIP-43 community admission keyed globally (admit-if-member-of-ANY-      *)
(*       community) instead of by (community, actor) -> Inv_AdmissionFence       *)
(*       breaks when a B-admitted actor joins or channel-less-reads in A.         *)
(*       Confirmed red: 5-state membership trace (Init -> WriteInsert ->         *)
(*       WriteInsert -> AdmitMember(commA, alice) -> AddMembership(commB)) and    *)
(*       4-state channel-less read trace (Init -> WriteInsert ->                 *)
(*       AdmitMember(commB, alice) -> ReadMessageRows(commA, NoChannel, hostA)). *)
(*   M10: open-community AUTH admission stamps a default/claimed community        *)
(*       instead of HostCommunity(host) -> Inv_AdmissionFence catches an          *)
(*       authRegistration whose host maps elsewhere. Confirmed red: 2-state      *)
(*       trace (Init -> AuthenticateOpenCommunity(hostB stamps commA)).           *)
(*   M11: channel creation stamps a default/claimed community instead of          *)
(*       HostCommunity(host) -> Inv_HostBindingFence /                           *)
(*       Inv_ChannelCommunityImmutable catches the fresh channel's bad owner.     *)
(*       Confirmed red: 2-state trace (Init -> CreateChannel(hostB stamps commA)).*)
(*   M12: no-#h host feed admission is relay-global (GloballyAdmitted(actor))     *)
(*       instead of by host community -> Inv_AdmissionFence catches a feed        *)
(*       read in A by an actor admitted only in B. Confirmed red: 3-state         *)
(*       trace (Init -> AdmitMember(commB, alice) -> ReadHostFeedRows(hostA)).    *)
(*   M13: no-#h #e-only aux admission is relay-global (GloballyAdmitted(actor))  *)
(*       instead of by host community -> Inv_AdmissionFence catches an aux        *)
(*       read in A by an actor admitted only in B. Confirmed red: 3-state         *)
(*       trace (Init -> AdmitMember(commB, alice) -> ReadHostAuxRows(hostA)).     *)
(***************************************************************************)
EXTENDS FiniteSets, Naturals, TLC

CONSTANTS
    Communities,       \* finite set of community ids
    Channels,          \* finite set of channel ids
    Hosts,             \* finite set of connection hostnames/URLs (the community selector)
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
    ChanFresh,         \* model value: initially-unregistered channel in TLC config
    HostA,             \* model value: host bound to community A
    HostB,             \* model value: host bound to community B
    HostBad,           \* model value: unmapped host (resolves to NoCommunity)
    NoChannel,         \* model value: sentinel channel for channel-less events
    NoCommunity,       \* model value: sentinel for an unmapped host (fail-closed)
    OpenCommunities,   \* communities with no NIP-43 member pubkey allowlist
    SanitizedErrors    \* fixed WebSocket-reachable sanitized error alphabet

ObsKinds == {"ResultRows", "WriteResult", "SanitizedError", "AuditHead", "AuthVerdict"}
MaxObservations == 2
WriteResults == {"Inserted", "Duplicate", "None"}
AuthVerdicts == {"Allow", "Deny", "None"}
NoError == "NoError"
NoAudit == "NoAudit"

InitialChannelOwners == [ch \in Channels |->
                         CASE ch \in {ChanA1, ChanA2} -> CommA
                           [] ch \in {ChanB1, ChanB2} -> CommB
                           [] OTHER                    -> NoCommunity]

\* resolve_host (P-RESOLVE-HOST): the connection's host is authoritative for the
\* community, exactly as a relay URL is authoritative for the relay today, lifted
\* one level up to community.  A host maps to exactly one community, or to the
\* NoCommunity sentinel (an unmapped/unknown host) -> the connection fails closed
\* and no channel-less write may derive a community from it.  This is the upstream
\* of ResolveTenant: ctx.community is *derived* from the host, never free-chosen.
HostCommunity == [h \in Hosts |->
                    CASE h = HostA -> CommA
                      [] h = HostB -> CommB
                      [] OTHER     -> NoCommunity]

\* Channel-or-sentinel: data rows and write/observation records may be channel-
\* less (channel = NoChannel), in which case the community comes from the host.
ChannelsExt == Channels \cup {NoChannel}

\* Every accepted write and recorded duplicate stamps its real originating host
\* (host \in Hosts): channel-less writes resolve the community from the host, and
\* channel-bearing writes/duplicates stamp the host on agreement (fail-closed on
\* disagreement, so no record carries a host that does not map to its community).
\* This lets Inv_HostBindingFence check that every record's stored community equals
\* its host's mapping -- i.e. the binding is enforced, not defaulted.

Symmetry ==
    Permutations(Actors) \cup
    Permutations(Workers) \cup
    Permutations(MsgIds) \cup
    Permutations(AuditVals)

VARIABLES
    messages,          \* set of canonical message rows (source="message")
    projections,       \* set of rebuildable projection rows (source="projection")
    memberships,       \* tenant-scoped active channel membership rows
    admittedMembers,   \* NIP-43 community member-npub allowlist rows
    channelLessReads,  \* successful channel-less read admissions (current capabilities)
    authRegistrations, \* open-community AUTH auto-registration witnesses
    feedReads,         \* no-#h kinds-only feed read witnesses
    auxReads,          \* no-#h #e-only aux read witnesses
    openChannels,      \* set of open/public channel ids
    auditHeads,        \* function: community -> current audit head
    observations,      \* typed outputs visible to tenant-scoped clients
    acceptedWrites,    \* write requests that inserted a new message row
    duplicateWrites,   \* write requests that no-op'd on scoped conflict
    createdChannels,   \* channel-create witnesses stamped from HostCommunity[host]
    queryFaults        \* fail-closed query-layer faults (e.g. no TenantContext)

vars == <<messages, projections, memberships, admittedMembers,
          channelLessReads, authRegistrations, feedReads, auxReads, openChannels,
          auditHeads, observations, acceptedWrites, duplicateWrites, createdChannels,
          queryFaults>>

ChannelCommunity(ch) ==
    IF InitialChannelOwners[ch] \in Communities THEN InitialChannelOwners[ch]
    ELSE IF \E created \in createdChannels : created.channel = ch
         THEN (CHOOSE created \in createdChannels : created.channel = ch).community
         ELSE NoCommunity

ChannelRegistered(ch) == ChannelCommunity(ch) \in Communities

\* ResolveTenant: the single resolver generalizing P-RESOLVE / L1 over both
\* channel-bearing and channel-less events.  For a channel-bearing op the
\* community is the server-owned channel mapping (and an h-cross-check requires it
\* to agree with the host community, see WriteInsert).  For a channel-less op
\* (channel = NoChannel) the community is the host-derived community.
ResolveTenant(host, ch) ==
    IF ch = NoChannel THEN HostCommunity[host] ELSE ChannelCommunity(ch)

DataRows == [
    id        : MsgIds,
    community : Communities,
    channel   : ChannelsExt,
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

AdmissionRows == [
    community : Communities,
    actor     : Actors
]

ChannelLessReadRows == [
    worker    : Workers,
    community : Communities,
    host      : Hosts,
    actor     : Actors
]

AuthRegistrationRows == [
    worker    : Workers,
    community : Communities,
    host      : Hosts,
    actor     : Actors
]

FeedReadRows == [
    worker    : Workers,
    community : Communities,
    host      : Hosts,
    actor     : Actors
]

AuxReadRows == [
    worker    : Workers,
    community : Communities,
    host      : Hosts,
    actor     : Actors,
    id        : MsgIds
]

CreatedChannelRows == [
    worker    : Workers,
    community : Communities,
    channel   : Channels,
    host      : Hosts,
    actor     : Actors
]

AcceptedWriteRows == [
    worker           : Workers,
    id               : MsgIds,
    community        : Communities,
    channel          : ChannelsExt,
    host             : Hosts,
    author           : Actors,
    claimedCommunity : Communities
]

DuplicateWriteRows == [
    worker           : Workers,
    id               : MsgIds,
    community        : Communities,
    channel          : ChannelsExt,
    host             : Hosts,
    author           : Actors,
    claimedCommunity : Communities
]

Observations == [
    worker    : Workers,
    actor     : Actors,
    community : Communities,          \* resolved/request TenantContext community
    channel   : ChannelsExt,          \* target channel; NoChannel for channel-less ops
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
    /\ InitialChannelOwners \in [Channels -> Communities \cup {NoCommunity}]
    /\ HostCommunity \in [Hosts -> Communities \cup {NoCommunity}]
    /\ OpenCommunities \subseteq Communities
    /\ CommA \in Communities
    /\ CommB \in Communities
    /\ CommA # CommB
    /\ {ChanA1, ChanA2, ChanB1, ChanB2, ChanFresh} \subseteq Channels
    /\ Cardinality({ChanA1, ChanA2, ChanB1, ChanB2, ChanFresh}) = 5
    /\ InitialChannelOwners[ChanFresh] = NoCommunity
    /\ ChanA1 # ChanB1
    /\ {HostA, HostB, HostBad} \subseteq Hosts
    /\ NoChannel \notin Channels
    /\ NoCommunity \notin Communities
    /\ HostCommunity[HostBad] = NoCommunity
    /\ SanitizedErrors # {}
    /\ NoError \notin SanitizedErrors
    /\ NoAudit \notin AuditVals
    /\ messages \subseteq MessageRows
    /\ projections \subseteq ProjectionRows
    /\ projections \subseteq DerivedProjectionRows
    /\ memberships \subseteq MembershipRows
    /\ admittedMembers \subseteq AdmissionRows
    /\ channelLessReads \subseteq ChannelLessReadRows
    /\ authRegistrations \subseteq AuthRegistrationRows
    /\ feedReads \subseteq FeedReadRows
    /\ auxReads \subseteq AuxReadRows
    /\ openChannels \subseteq Channels
    /\ openChannels \subseteq {ch \in Channels : ChannelRegistered(ch)}
    /\ auditHeads \in [Communities -> AuditVals]
    /\ observations \subseteq Observations
    /\ acceptedWrites \subseteq AcceptedWriteRows
    /\ duplicateWrites \subseteq DuplicateWriteRows
    /\ createdChannels \subseteq CreatedChannelRows
    /\ queryFaults \subseteq QueryFaultRows
    \* Tenant-scoped control plane: a membership row's community agrees with
    \* the server-owned channel -> community mapping. Memberships are always
    \* channel-scoped (no channel-less memberships).
    /\ \A m \in memberships : /\ ChannelRegistered(m.channel)
                              /\ m.community = ChannelCommunity(m.channel)
    \* Message/projection rows are stamped with the resolved community: the
    \* server-owned channel mapping for channel-bearing rows, and a real
    \* (never-sentinel) community for channel-less rows (host-resolved at write).
    /\ \A m \in messages :
        IF m.channel = NoChannel THEN m.community \in Communities
                                 ELSE /\ ChannelRegistered(m.channel)
                                      /\ m.community = ChannelCommunity(m.channel)
    /\ \A p \in projections :
        IF p.channel = NoChannel THEN p.community \in Communities
                                 ELSE /\ ChannelRegistered(p.channel)
                                      /\ p.community = ChannelCommunity(p.channel)

Init ==
    /\ messages = {}
    /\ projections = {}
    /\ memberships = {}
    /\ admittedMembers = {}
    /\ channelLessReads = {}
    /\ authRegistrations = {}
    /\ feedReads = {}
    /\ auxReads = {}
    /\ openChannels = {}
    /\ auditHeads \in [Communities -> AuditVals]
    /\ observations = {}
    /\ acceptedWrites = {}
    /\ duplicateWrites = {}
    /\ createdChannels = {}
    /\ queryFaults = {}

IsAdmitted(community, actor) ==
    [community |-> community, actor |-> actor] \in admittedMembers

\* Intentionally-bad admission helper for mutation M9: treats the NIP-43 member
\* allowlist as relay-global by dropping community from the key.  Substitute this
\* for IsAdmitted in AddMembership / channel-less reads to reproduce M9.
GloballyAdmitted(actor) ==
    \E c \in Communities : [community |-> c, actor |-> actor] \in admittedMembers

\* Open-community AUTH mutation M10 helper: stamp every open AUTH registration
\* into CommA, ignoring the authoritative host. To reproduce M10, substitute
\* DefaultOpenAuthCommunity(host) for HostCommunity[host] when building row/reg
\* in AuthenticateOpenCommunity; HostB then records a commA registration and
\* Inv_AdmissionFence fails in a 2-state trace.
DefaultOpenAuthCommunity(host) == CommA

\* Channel-creation mutation M11 helper: stamp every fresh channel into CommA,
\* ignoring the authoritative host. To reproduce M11, substitute
\* DefaultCreatedChannelCommunity(host) for HostCommunity[host] in CreateChannel;
\* HostB then creates a commA-owned channel and the host/immutability fences fail.
DefaultCreatedChannelCommunity(host) == CommA

ScopedAccessible(community, actor) ==
    {ch \in Channels :
        /\ ChannelCommunity(ch) = community
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

VisibleMessageRows(community, actor, targetChannel) ==
    {m \in messages :
        /\ m.community = community
        /\ IF targetChannel = NoChannel
           THEN /\ m.channel = NoChannel
                /\ IsAdmitted(community, actor)
           ELSE m.channel \in ScopedAccessible(community, actor)}

VisibleProjectionRows(community, actor, targetChannel) ==
    {p \in projections :
        /\ p.community = community
        /\ IF targetChannel = NoChannel
           THEN /\ p.channel = NoChannel
                /\ IsAdmitted(community, actor)
           ELSE p.channel \in ScopedAccessible(community, actor)}

VisibleDirectIdRows(community, actor, id, targetChannel) ==
    {m \in messages :
        /\ m.id = id
        /\ m.community = community
        /\ IF targetChannel = NoChannel
           THEN /\ m.channel = NoChannel
                /\ IsAdmitted(community, actor)
           ELSE m.channel \in ScopedAccessible(community, actor)}

VisibleHostFeedRows(community, actor) ==
    {m \in messages :
        /\ m.community = community
        /\ IsAdmitted(community, actor)
        /\ (m.channel = NoChannel \/ m.channel \in ScopedAccessible(community, actor))}

\* Intentionally-bad host feed helper for mutation M12: answers from the host
\* community but treats admission as relay-global. Substitute this into
\* ReadHostFeedRows; after AdmitMember(commB, actor), a HostA feed read is recorded
\* for commA and Inv_AdmissionFence fails.
VisibleHostFeedRows_GlobalAdmission(community, actor) ==
    {m \in messages :
        /\ m.community = community
        /\ GloballyAdmitted(actor)
        /\ (m.channel = NoChannel \/ m.channel \in ScopedAccessible(community, actor))}

VisibleHostAuxRows(community, actor, id) ==
    {m \in messages :
        /\ m.id = id
        /\ m.community = community
        /\ IsAdmitted(community, actor)
        /\ (m.channel = NoChannel \/ m.channel \in ScopedAccessible(community, actor))}

\* Intentionally-bad host aux helper for mutation M13: answers from the host
\* community but treats admission as relay-global. Substitute this into
\* ReadHostAuxRows; after AdmitMember(commB, actor), a HostA aux read is recorded
\* for commA and Inv_AdmissionFence fails.
VisibleHostAuxRows_GlobalAdmission(community, actor, id) ==
    {m \in messages :
        /\ m.id = id
        /\ m.community = community
        /\ GloballyAdmitted(actor)
        /\ (m.channel = NoChannel \/ m.channel \in ScopedAccessible(community, actor))}

\* Intentionally-bad direct lookup mutation: answer by global id first and trust
\* the row's own community as scope.  Substitute this into ReadByIdRows for M7.
UnscopedDirectIdRows(actor, id) ==
    {m \in messages :
        /\ m.id = id
        /\ m.channel \in UnscopedAccessible(actor)}

RLSRows(community, rows) == {r \in rows : r.community = community}

\* Channel-bearing write.  The community is resolved server-side from the h tag
\* (real == ChannelCommunity(ch)), never the claimed community.  But the host is
\* *also* authoritative: an A-host connection presenting a B-channel event is a
\* confused deputy on the host axis (URL-is-community is the admission boundary),
\* so the op fails closed unless HostCommunity[host] = ChannelCommunity(ch).
\* Disagreement (incl. an unmapped HostBad whose mapping is NoCommunity, which can
\* never equal a real channel community) writes nothing, emits a sanitized error,
\* and records a query fault.  Never acts as the channel's community.
WriteInsert(w) ==
    /\ Cardinality(observations) < MaxObservations
    /\ \E id \in MsgIds, ch \in Channels, host \in Hosts, a \in Actors, claimed \in Communities :
        LET real == ChannelCommunity(ch)
        IN
            IF ~(real \in Communities) \/ HostCommunity[host] # real
            THEN \* fail-closed: unknown channel or host/channel disagreement (A-host + B-channel).
                LET obs == [worker |-> w, actor |-> a, community |-> claimed,
                            channel |-> ch, kind |-> "SanitizedError",
                            labels |-> {}, rows |-> {}, error |-> "restricted",
                            result |-> "None", verdict |-> "None", audit |-> NoAudit]
                    fault == [worker |-> w, actor |-> a, community |-> claimed,
                              reason |-> "missing_tenant_context"]
                IN
                    /\ observations' = observations \cup {obs}
                    /\ queryFaults' = queryFaults \cup {fault}
                    /\ UNCHANGED <<messages, projections, memberships, admittedMembers,
                                  channelLessReads, authRegistrations, feedReads, auxReads,
                                  openChannels, auditHeads, acceptedWrites, duplicateWrites,
                                  createdChannels>>
            ELSE \* host agrees with the channel mapping: accept, stamp the host.
                LET key == [community |-> real, id |-> id]
                    row == MessageRow(id, real, ch, a)
                    obs == [worker |-> w, actor |-> a, community |-> real, channel |-> ch,
                            kind |-> "WriteResult", labels |-> {real}, rows |-> {row},
                            error |-> NoError, result |-> "Inserted", verdict |-> "None", audit |-> NoAudit]
                    wr  == [worker |-> w, id |-> id, community |-> real,
                            channel |-> ch, host |-> host, author |-> a, claimedCommunity |-> claimed]
                IN
                    /\ key \notin MessageKeys
                    /\ messages' = messages \cup {row}
                    /\ observations' = observations \cup {obs}
                    /\ acceptedWrites' = acceptedWrites \cup {wr}
                    /\ UNCHANGED <<projections, memberships, admittedMembers, channelLessReads,
                                  authRegistrations, feedReads, auxReads, openChannels, auditHeads,
                                  duplicateWrites, createdChannels, queryFaults>>

\* Channel-less write (kind:0 profiles, 1059 DMs, 30023/30174/30315/30078, lists).
\* There is no h tag to resolve, so the community is derived from the connection's
\* host (P-RESOLVE-HOST / ResolveTenant): the row is stamped channel = NoChannel,
\* community = HostCommunity[host].  The claimed community is adversary-controlled
\* and ignored -- host wins (the confused-deputy fence, lifted to the host).
\* An unmapped host (HostCommunity[host] = NoCommunity) fails closed: no row is
\* written, the connection sees only a sanitized error, and a query fault is
\* recorded.  Never a default tenant.
WriteInsertGlobal(w) ==
    /\ Cardinality(observations) < MaxObservations
    /\ \E id \in MsgIds, host \in Hosts, a \in Actors, claimed \in Communities :
        LET resolved == HostCommunity[host]
        IN
            IF resolved = NoCommunity \/ ~IsAdmitted(resolved, a)
            THEN \* fail-closed: unmapped/unknown host or community admission miss.
                LET obs == [worker |-> w, actor |-> a, community |-> claimed,
                            channel |-> NoChannel, kind |-> "SanitizedError",
                            labels |-> {}, rows |-> {}, error |-> "restricted",
                            result |-> "None", verdict |-> "None", audit |-> NoAudit]
                    fault == [worker |-> w, actor |-> a, community |-> claimed,
                              reason |-> "missing_tenant_context"]
                IN
                    /\ observations' = observations \cup {obs}
                    /\ queryFaults' = queryFaults \cup {fault}
                    /\ UNCHANGED <<messages, projections, memberships, admittedMembers,
                                  channelLessReads, authRegistrations, feedReads, auxReads,
                                  openChannels, auditHeads, acceptedWrites, duplicateWrites,
                                  createdChannels>>
            ELSE \* mapped host: stamp the host-resolved community, channel-less.
                LET key == [community |-> resolved, id |-> id]
                    row == [id |-> id, community |-> resolved, channel |-> NoChannel,
                            author |-> a, source |-> "message"]
                    obs == [worker |-> w, actor |-> a, community |-> resolved,
                            channel |-> NoChannel, kind |-> "WriteResult",
                            labels |-> {resolved}, rows |-> {row}, error |-> NoError,
                            result |-> "Inserted", verdict |-> "None", audit |-> NoAudit]
                    wr  == [worker |-> w, id |-> id, community |-> resolved,
                            channel |-> NoChannel, host |-> host, author |-> a, claimedCommunity |-> claimed]
                IN
                    /\ key \notin MessageKeys
                    /\ messages' = messages \cup {row}
                    /\ observations' = observations \cup {obs}
                    /\ acceptedWrites' = acceptedWrites \cup {wr}
                    /\ UNCHANGED <<projections, memberships, admittedMembers, channelLessReads,
                                  authRegistrations, feedReads, auxReads, openChannels, auditHeads,
                                  duplicateWrites, createdChannels, queryFaults>>

\* Duplicate / no-op write outcome (scoped conflict on (community_id, id)).  This
\* is client-observable write surface -- the "Duplicate" WriteResult exposes the
\* scoped existence/conflict rows -- so it carries the SAME host-axis obligation as
\* WriteInsert.  The community is resolved server-side from the h tag (real ==
\* ChannelCommunity(ch)); the host is *also* authoritative.  An A-host presenting a
\* B-channel id is a confused deputy on the host axis: it must NOT learn whether
\* that id already exists in B.  So the op fails closed (sanitized error + query
\* fault, no Duplicate result, nothing recorded) unless HostCommunity[host] = real.
WriteDuplicate(w) ==
    /\ Cardinality(observations) < MaxObservations
    /\ \E id \in MsgIds, ch \in Channels, host \in Hosts, a \in Actors, claimed \in Communities :
        LET real == ChannelCommunity(ch)
        IN
            IF ~(real \in Communities) \/ HostCommunity[host] # real
            THEN \* fail-closed: unknown channel or host/channel disagreement (A-host + B-channel).
                LET obs == [worker |-> w, actor |-> a, community |-> claimed,
                            channel |-> ch, kind |-> "SanitizedError",
                            labels |-> {}, rows |-> {}, error |-> "restricted",
                            result |-> "None", verdict |-> "None", audit |-> NoAudit]
                    fault == [worker |-> w, actor |-> a, community |-> claimed,
                              reason |-> "missing_tenant_context"]
                IN
                    /\ observations' = observations \cup {obs}
                    /\ queryFaults' = queryFaults \cup {fault}
                    /\ UNCHANGED <<messages, projections, memberships, admittedMembers,
                                  channelLessReads, authRegistrations, feedReads, auxReads,
                                  openChannels, auditHeads, acceptedWrites, duplicateWrites,
                                  createdChannels>>
            ELSE \* host agrees with the channel mapping: report the scoped duplicate.
                LET key == [community |-> real, id |-> id]
                    conflicts == ScopedConflictRows(real, id)
                    obs == [worker |-> w, actor |-> a, community |-> real, channel |-> ch,
                            kind |-> "WriteResult", labels |-> RowLabels(conflicts), rows |-> conflicts,
                            error |-> NoError, result |-> "Duplicate", verdict |-> "None", audit |-> NoAudit]
                    wr  == [worker |-> w, id |-> id, community |-> real,
                            channel |-> ch, host |-> host, author |-> a, claimedCommunity |-> claimed]
                IN
                    /\ key \in MessageKeys
                    /\ messages' = messages
                    /\ observations' = observations \cup {obs}
                    /\ duplicateWrites' = duplicateWrites \cup {wr}
                    /\ UNCHANGED <<projections, memberships, admittedMembers, channelLessReads,
                                  authRegistrations, feedReads, auxReads, openChannels, auditHeads,
                                  acceptedWrites, createdChannels, queryFaults>>

ReadMessageRows(w) ==
    /\ Cardinality(observations) < MaxObservations
    /\ \E c \in Communities, a \in Actors, ch \in ChannelsExt, host \in Hosts :
        LET rows == VisibleMessageRows(c, a, ch)
            obs == [worker |-> w, actor |-> a, community |-> c, channel |-> ch,
                    kind |-> "ResultRows", labels |-> RowLabels(rows), rows |-> rows,
                    error |-> NoError, result |-> "None", verdict |-> "None", audit |-> NoAudit]
            read == [worker |-> w, community |-> c, host |-> host, actor |-> a]
        IN
            /\ IF ch = NoChannel
               THEN /\ HostCommunity[host] = c
                    /\ IsAdmitted(c, a)
                    /\ channelLessReads' = channelLessReads \cup {read}
               ELSE channelLessReads' = channelLessReads
            /\ observations' = observations \cup {obs}
            /\ UNCHANGED <<messages, projections, memberships, admittedMembers,
                          authRegistrations, feedReads, auxReads, openChannels, auditHeads,
                          acceptedWrites, duplicateWrites, createdChannels, queryFaults>>

ReadProjectionRows(w) ==
    /\ Cardinality(observations) < MaxObservations
    /\ \E c \in Communities, a \in Actors, ch \in ChannelsExt, host \in Hosts :
        \E rows \in SUBSET VisibleProjectionRows(c, a, ch) :
            LET obs == [worker |-> w, actor |-> a, community |-> c, channel |-> ch,
                        kind |-> "ResultRows", labels |-> RowLabels(rows), rows |-> rows,
                        error |-> NoError, result |-> "None", verdict |-> "None", audit |-> NoAudit]
                read == [worker |-> w, community |-> c, host |-> host, actor |-> a]
            IN
                /\ IF ch = NoChannel
                   THEN /\ HostCommunity[host] = c
                        /\ IsAdmitted(c, a)
                        /\ channelLessReads' = channelLessReads \cup {read}
                   ELSE channelLessReads' = channelLessReads
                /\ observations' = observations \cup {obs}
                /\ UNCHANGED <<messages, projections, memberships, admittedMembers,
                              authRegistrations, feedReads, auxReads, openChannels, auditHeads,
                              acceptedWrites, duplicateWrites, createdChannels, queryFaults>>

ReadByIdRows(w) ==
    /\ Cardinality(observations) < MaxObservations
    /\ \E c \in Communities, a \in Actors, ch \in ChannelsExt, host \in Hosts, id \in MsgIds :
        LET rows == VisibleDirectIdRows(c, a, id, ch)
            obs == [worker |-> w, actor |-> a, community |-> c, channel |-> ch,
                    kind |-> "ResultRows", labels |-> RowLabels(rows), rows |-> rows,
                    error |-> NoError, result |-> "None", verdict |-> "None", audit |-> NoAudit]
            read == [worker |-> w, community |-> c, host |-> host, actor |-> a]
        IN
            /\ IF ch = NoChannel
               THEN /\ HostCommunity[host] = c
                    /\ IsAdmitted(c, a)
                    /\ channelLessReads' = channelLessReads \cup {read}
               ELSE channelLessReads' = channelLessReads
            /\ observations' = observations \cup {obs}
            /\ UNCHANGED <<messages, projections, memberships, admittedMembers,
                          authRegistrations, feedReads, auxReads, openChannels, auditHeads,
                          acceptedWrites, duplicateWrites, createdChannels, queryFaults>>

\* No-#h kinds-only feed/global read. The client sends no community metadata; the
\* relay derives the community from the connection host and fans out across only
\* channel-less rows plus accessible channels inside that same host community.
ReadHostFeedRows(w) ==
    /\ Cardinality(observations) < MaxObservations
    /\ \E host \in Hosts, a \in Actors :
        LET c == HostCommunity[host]
        IN
            /\ c \in Communities
            /\ IsAdmitted(c, a)
            /\ [worker |-> w, community |-> c, host |-> host, actor |-> a] \notin feedReads
            /\ LET rows == VisibleHostFeedRows(c, a)
                   obs == [worker |-> w, actor |-> a, community |-> c, channel |-> NoChannel,
                           kind |-> "ResultRows", labels |-> RowLabels(rows), rows |-> rows,
                           error |-> NoError, result |-> "None", verdict |-> "None", audit |-> NoAudit]
                   read == [worker |-> w, community |-> c, host |-> host, actor |-> a]
               IN
                   /\ observations' = observations \cup {obs}
                   /\ feedReads' = feedReads \cup {read}
                   /\ UNCHANGED <<messages, projections, memberships, admittedMembers,
                                 channelLessReads, authRegistrations, auxReads, openChannels,
                                 auditHeads, acceptedWrites, duplicateWrites, createdChannels, queryFaults>>

\* No-#h #e-only aux lookup (reactions/edits/deletes/thread metadata shape). It
\* resolves by id only after applying the host-community and accessible-channel
\* fences, so same event id in another community is invisible.
ReadHostAuxRows(w) ==
    /\ Cardinality(observations) < MaxObservations
    /\ \E host \in Hosts, a \in Actors, id \in MsgIds :
        LET c == HostCommunity[host]
        IN
            /\ c \in Communities
            /\ IsAdmitted(c, a)
            /\ [worker |-> w, community |-> c, host |-> host, actor |-> a, id |-> id] \notin auxReads
            /\ LET rows == VisibleHostAuxRows(c, a, id)
                   obs == [worker |-> w, actor |-> a, community |-> c, channel |-> NoChannel,
                           kind |-> "ResultRows", labels |-> RowLabels(rows), rows |-> rows,
                           error |-> NoError, result |-> "None", verdict |-> "None", audit |-> NoAudit]
                   read == [worker |-> w, community |-> c, host |-> host, actor |-> a, id |-> id]
               IN
                   /\ observations' = observations \cup {obs}
                   /\ auxReads' = auxReads \cup {read}
                   /\ UNCHANGED <<messages, projections, memberships, admittedMembers,
                                 channelLessReads, authRegistrations, feedReads, openChannels,
                                 auditHeads, acceptedWrites, duplicateWrites, createdChannels, queryFaults>>

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
            /\ UNCHANGED <<messages, projections, memberships, admittedMembers, channelLessReads,
                          authRegistrations, feedReads, auxReads, openChannels, auditHeads,
                          acceptedWrites, duplicateWrites, createdChannels, queryFaults>>

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
            /\ UNCHANGED <<messages, projections, memberships, admittedMembers, channelLessReads,
                          authRegistrations, feedReads, auxReads, openChannels, auditHeads,
                          acceptedWrites, duplicateWrites, createdChannels>>

SanitizedError(w) ==
    /\ Cardinality(observations) < MaxObservations
    /\ \E c \in Communities, a \in Actors, ch \in Channels, e \in SanitizedErrors :
        LET obs == [worker |-> w, actor |-> a, community |-> c, channel |-> ch,
                    kind |-> "SanitizedError", labels |-> {}, rows |-> {},
                    error |-> e, result |-> "None", verdict |-> "None", audit |-> NoAudit]
        IN
            /\ observations' = observations \cup {obs}
            /\ UNCHANGED <<messages, projections, memberships, admittedMembers, channelLessReads,
                          authRegistrations, feedReads, auxReads, openChannels, auditHeads,
                          acceptedWrites, duplicateWrites, createdChannels, queryFaults>>

\* Channel-bearing authorization.  Community resolves from the channel mapping;
\* the host is also authoritative.  Host/channel disagreement (A-host + B-channel)
\* is denied -- the host axis of the confused-deputy fence.  Even if the actor is
\* a member of the B-channel, an A-host connection cannot drive a B-channel verdict.
AuthCheck(w) ==
    /\ Cardinality(observations) < MaxObservations
    /\ \E ch \in Channels, host \in Hosts, a \in Actors, claimed \in Communities :
        LET real == ChannelCommunity(ch)
            hostAgrees == real \in Communities /\ HostCommunity[host] = real
            allowed == hostAgrees /\ ch \in ScopedAccessible(real, a)
            verdict == IF allowed THEN "Allow" ELSE "Deny"
            obs == [worker |-> w, actor |-> a, community |-> real, channel |-> ch,
                    kind |-> "AuthVerdict", labels |-> {real}, rows |-> {},
                    error |-> NoError, result |-> "None", verdict |-> verdict, audit |-> NoAudit]
        IN
            /\ real \in Communities
            /\ observations' = observations \cup {obs}
            /\ UNCHANGED <<messages, projections, memberships, admittedMembers, channelLessReads,
                          authRegistrations, feedReads, auxReads, openChannels, auditHeads,
                          acceptedWrites, duplicateWrites, createdChannels, queryFaults>>

AppendAudit(w) ==
    \E c \in Communities, newHead \in AuditVals :
        /\ newHead # auditHeads[c]
        /\ auditHeads' = [auditHeads EXCEPT ![c] = newHead]
        /\ UNCHANGED <<messages, projections, memberships, admittedMembers, channelLessReads,
                      authRegistrations, feedReads, auxReads, openChannels, observations,
                      acceptedWrites, duplicateWrites, createdChannels, queryFaults>>

ObserveAuditHead(w) ==
    /\ Cardinality(observations) < MaxObservations
    /\ \E c \in Communities, a \in Actors, ch \in Channels :
        LET obs == [worker |-> w, actor |-> a, community |-> c, channel |-> ch,
                    kind |-> "AuditHead", labels |-> {c}, rows |-> {},
                    error |-> NoError, result |-> "None", verdict |-> "None", audit |-> auditHeads[c]]
        IN
            /\ observations' = observations \cup {obs}
            /\ UNCHANGED <<messages, projections, memberships, admittedMembers, channelLessReads,
                          authRegistrations, feedReads, auxReads, openChannels, auditHeads,
                          acceptedWrites, duplicateWrites, createdChannels, queryFaults>>

\* Projection rebuild is privileged internal work. It may touch all communities
\* and may leave projections temporarily partial, but it emits no observation.
RebuildProjections(w) ==
    \E rebuilt \in SUBSET DerivedProjectionRows :
        /\ projections' = rebuilt
        /\ UNCHANGED <<messages, memberships, admittedMembers, channelLessReads,
                      authRegistrations, feedReads, auxReads, openChannels, auditHeads, observations,
                      acceptedWrites, duplicateWrites, createdChannels, queryFaults>>

\* AUTH to an allowlist-less/open community auto-registers the authenticated npub
\* in that host-derived community. Later reads/writes still check IsAdmitted; the
\* row exists because AUTH inserted it, not because the read path bypassed admission.
AuthenticateOpenCommunity(w) ==
    \E host \in Hosts, a \in Actors :
        LET c == HostCommunity[host]
            row == [community |-> c, actor |-> a]
            reg == [worker |-> w, community |-> c, host |-> host, actor |-> a]
        IN
            /\ c \in OpenCommunities
            /\ row \notin admittedMembers
            /\ admittedMembers' = admittedMembers \cup {row}
            /\ authRegistrations' = authRegistrations \cup {reg}
            /\ UNCHANGED <<messages, projections, memberships, channelLessReads,
                          feedReads, auxReads, openChannels, auditHeads, observations,
                          acceptedWrites, duplicateWrites, createdChannels, queryFaults>>

\* Channel creation is host-bound: a fresh channel has no community until the
\* relay accepts kind:9007 on a mapped host, then stamps HostCommunity[host]
\* atomically. The client sends no community id/tag.
CreateChannel(w) ==
    \E ch \in Channels, host \in Hosts, a \in Actors :
        LET c == HostCommunity[host]
            created == [worker |-> w, community |-> c, channel |-> ch, host |-> host, actor |-> a]
        IN
            /\ c \in Communities
            /\ ChannelCommunity(ch) = NoCommunity
            /\ ~\E old \in createdChannels : old.channel = ch
            /\ createdChannels' = createdChannels \cup {created}
            /\ UNCHANGED <<messages, projections, memberships, admittedMembers, channelLessReads,
                          authRegistrations, feedReads, auxReads, openChannels, auditHeads,
                          observations, acceptedWrites, duplicateWrites, queryFaults>>

AdmitMember(w) ==
    \E c \in Communities, a \in Actors :
        LET row == [community |-> c, actor |-> a]
        IN
            /\ admittedMembers' = admittedMembers \cup {row}
            /\ UNCHANGED <<messages, projections, memberships, channelLessReads,
                          authRegistrations, feedReads, auxReads, openChannels, auditHeads,
                          observations, acceptedWrites, duplicateWrites, createdChannels, queryFaults>>

RevokeMember(w) ==
    \E c \in Communities, a \in Actors :
        LET row == [community |-> c, actor |-> a]
        IN
            /\ admittedMembers' = admittedMembers \ {row}
            /\ memberships' = {m \in memberships : ~(m.community = c /\ m.actor = a)}
            /\ channelLessReads' = {r \in channelLessReads : ~(r.community = c /\ r.actor = a)}
            /\ feedReads' = {r \in feedReads : ~(r.community = c /\ r.actor = a)}
            /\ auxReads' = {r \in auxReads : ~(r.community = c /\ r.actor = a)}
            /\ UNCHANGED <<messages, projections, authRegistrations,
                          openChannels, auditHeads, observations, acceptedWrites,
                          duplicateWrites, createdChannels, queryFaults>>

AddMembership(w) ==
    \E ch \in Channels, a \in Actors :
        LET c == ChannelCommunity(ch)
            row == [community |-> c, channel |-> ch, actor |-> a]
        IN
            /\ c \in Communities
            /\ IsAdmitted(c, a)
            /\ memberships' = memberships \cup {row}
            /\ UNCHANGED <<messages, projections, admittedMembers, channelLessReads,
                          authRegistrations, feedReads, auxReads, openChannels, auditHeads,
                          observations, acceptedWrites, duplicateWrites, createdChannels, queryFaults>>

RemoveMembership(w) ==
    \E ch \in Channels, a \in Actors :
        LET c == ChannelCommunity(ch)
            row == [community |-> c, channel |-> ch, actor |-> a]
        IN
            /\ c \in Communities
            /\ memberships' = memberships \ {row}
            /\ UNCHANGED <<messages, projections, admittedMembers, channelLessReads,
                          authRegistrations, feedReads, auxReads, openChannels, auditHeads,
                          observations, acceptedWrites, duplicateWrites, createdChannels, queryFaults>>

OpenChannel(w) ==
    \E ch \in Channels :
        /\ ChannelRegistered(ch)
        /\ openChannels' = openChannels \cup {ch}
        /\ UNCHANGED <<messages, projections, memberships, admittedMembers, channelLessReads,
                      authRegistrations, feedReads, auxReads, auditHeads, observations,
                      acceptedWrites, duplicateWrites, createdChannels, queryFaults>>

CloseChannel(w) ==
    \E ch \in Channels :
        /\ openChannels' = openChannels \ {ch}
        /\ UNCHANGED <<messages, projections, memberships, admittedMembers, channelLessReads,
                      authRegistrations, feedReads, auxReads, auditHeads, observations,
                      acceptedWrites, duplicateWrites, createdChannels, queryFaults>>

Next ==
    \E w \in Workers :
        \/ WriteInsert(w)
        \/ WriteInsertGlobal(w)
        \/ WriteDuplicate(w)
        \/ ReadMessageRows(w)
        \/ ReadProjectionRows(w)
        \/ ReadByIdRows(w)
        \/ ReadHostFeedRows(w)
        \/ ReadHostAuxRows(w)
        \/ ReadForgotPredicateWithRLS(w)
        \/ ReadNoTenantContext(w)
        \/ SanitizedError(w)
        \/ AuthCheck(w)
        \/ AppendAudit(w)
        \/ ObserveAuditHead(w)
        \/ RebuildProjections(w)
        \/ AuthenticateOpenCommunity(w)
        \/ CreateChannel(w)
        \/ AdmitMember(w)
        \/ RevokeMember(w)
        \/ AddMembership(w)
        \/ RemoveMembership(w)
        \/ OpenChannel(w)
        \/ CloseChannel(w)

BoundedObservations == Cardinality(observations) <= MaxObservations

BoundedWitnesses ==
    /\ Cardinality(messages) <= 1
    /\ Cardinality(projections) <= 1
    /\ Cardinality(memberships) <= 1
    /\ Cardinality(admittedMembers) <= 2
    /\ Cardinality(channelLessReads) <= 1
    /\ Cardinality(authRegistrations) <= 1
    /\ Cardinality(feedReads) <= 1
    /\ Cardinality(auxReads) <= 1
    /\ Cardinality(openChannels) <= 1
    /\ Cardinality(acceptedWrites) <= 1
    /\ Cardinality(duplicateWrites) <= 1
    /\ Cardinality(createdChannels) <= 1
    /\ Cardinality(queryFaults) <= 1

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
        /\ (o.kind = "AuthVerdict" => o.labels = {ChannelCommunity(o.channel)} /\ o.community = ChannelCommunity(o.channel))

\* I1 read confinement follows from NI + label propagation, but is kept as a
\* legible mutation target for the current get_accessible_channel_ids landmine.
Inv_ReadConfinement ==
    \A o \in observations :
        o.kind = "ResultRows" => \A r \in o.rows : r.community = o.community

\* I2 resolution fence: persisted messages and write/auth observations are
\* labeled by the *resolved* community, never a client-supplied claim.  For a
\* channel-bearing op the resolver is the server-owned channel->community mapping
\* (the h tag is not the source).  For a channel-less op (channel = NoChannel) the
\* resolver is the connection's host (P-RESOLVE-HOST): the stamped community is a
\* real community derived from the host, never the NoCommunity sentinel and never
\* the adversary-controlled claimedCommunity.  At N=1 (one host -> one community)
\* the two branches coincide.
Inv_ResolutionFence ==
    /\ \A m \in messages :
        IF m.channel = NoChannel THEN m.community \in Communities
                                 ELSE /\ ChannelRegistered(m.channel)
                                      /\ m.community = ChannelCommunity(m.channel)
    /\ \A w \in acceptedWrites :
        IF w.channel = NoChannel THEN w.community \in Communities
                                 ELSE w.community = ChannelCommunity(w.channel)
    /\ \A w \in duplicateWrites :
        IF w.channel = NoChannel THEN w.community \in Communities
                                 ELSE w.community = ChannelCommunity(w.channel)
    /\ \A o \in observations :
        o.kind \in {"WriteResult", "AuthVerdict"} =>
            (IF o.channel = NoChannel THEN o.community \in Communities
                                      ELSE o.community = ChannelCommunity(o.channel))

\* I2-host fail-closed binding: EVERY accepted write -- channel-bearing or
\* channel-less -- AND every observable duplicate/no-op outcome carries the host it
\* came in on, and that host's mapping is a real community equal to the write's
\* stored community (P-RESOLVE-HOST).  For channel-less writes this is the
\* host->community derivation; for channel-bearing writes and duplicate/no-op
\* outcomes it is the host/channel agreement fence (HostCommunity[host] =
\* ChannelCommunity(ch)).  This makes BOTH "an unmapped host defaults to a tenant"
\* and "an A-host can drive a B-channel op (insert OR duplicate-probe)" CAUGHT
\* mutations: the fail-closed branches of WriteInsert/WriteInsertGlobal/WriteDuplicate
\* write/record nothing on disagreement, so no accepted write and no recorded
\* duplicate can carry a host whose mapping is NoCommunity or disagrees with the stamp.
Inv_HostBindingFence ==
    /\ \A w \in acceptedWrites :
        /\ w.host \in Hosts
        /\ HostCommunity[w.host] \in Communities
        /\ w.community = HostCommunity[w.host]
    /\ \A w \in duplicateWrites :
        /\ w.host \in Hosts
        /\ HostCommunity[w.host] \in Communities
        /\ w.community = HostCommunity[w.host]
    /\ \A ch \in createdChannels :
        /\ ch.host \in Hosts
        /\ HostCommunity[ch.host] \in Communities
        /\ ch.community = HostCommunity[ch.host]
        /\ ChannelCommunity(ch.channel) = ch.community

\* Channel-community immutability: a runtime-created channel may be stamped at
\* most once, and all durable ownership rows for a channel agree on the same
\* host-resolved community. This makes the old constant-function immutability
\* guarantee explicit now that fresh channel creation is modeled as state.
Inv_ChannelCommunityImmutable ==
    /\ \A c1, c2 \in createdChannels :
        c1.channel = c2.channel => c1.community = c2.community
    /\ \A created \in createdChannels :
        /\ InitialChannelOwners[created.channel] = NoCommunity
        /\ created.community = HostCommunity[created.host]

\* I2-admission fence: NIP-43 member-npub admission is scoped by community, not
\* relay-global pubkey.  Active channel memberships and current channel-less read
\* capabilities may only exist when the actor is admitted in that same community;
\* channel-less read capabilities also carry the host that resolved that community.
\* This records the observable admission capability, not historical message rows:
\* revoking admission removes current memberships/reads but does not relabel rows
\* already written while admitted.
Inv_AdmissionFence ==
    /\ \A m \in memberships : IsAdmitted(m.community, m.actor)
    /\ \A r \in channelLessReads :
        /\ IsAdmitted(r.community, r.actor)
        /\ HostCommunity[r.host] = r.community
    /\ \A r \in authRegistrations :
        /\ r.community \in OpenCommunities
        /\ HostCommunity[r.host] = r.community
    /\ \A r \in feedReads :
        /\ IsAdmitted(r.community, r.actor)
        /\ HostCommunity[r.host] = r.community
    /\ \A r \in auxReads :
        /\ IsAdmitted(r.community, r.actor)
        /\ HostCommunity[r.host] = r.community

\* TLA-side anti-vacuity / reachability witnesses for surfaces that could
\* otherwise pass only because their actions never fire. These are intentionally
\* false invariants: checking one ad hoc with INVARIANT (plus the normal
\* constraints) must produce a short counterexample trace. If it stays green, the
\* corresponding action is unreachable and the safety proof is vacuous there.
Probe_OpenAuthRegistration_Unreachable ==
    authRegistrations = {}

Probe_CreatedChannel_Unreachable ==
    createdChannels = {}

Probe_HostFeedRead_Unreachable ==
    feedReads = {}

Probe_HostAuxRead_Unreachable ==
    auxReads = {}

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
    /\ Inv_HostBindingFence
    /\ Inv_ChannelCommunityImmutable
    /\ Inv_AdmissionFence
    /\ Inv_AcceptedWritesPersist
    /\ Inv_MessageKeyUnique
    /\ Inv_NoTenantContextFailsClosed
    /\ Inv_ProjectionDerived
    /\ Inv_SanitizedErrors
=============================================================================
