---------------------------- MODULE GitGovernance ----------------------------
(***************************************************************************)
(* Who may change a governed Git repository, what proof a change needs,    *)
(* who may see it, and what record it leaves.  VISION_GOVERNANCE.md states *)
(* the contract in prose; this module states it precisely enough to check. *)
(*                                                                         *)
(*   community admins ──► floor, protected labels, trusted checks          *)
(*   repo admins      ──► repo rules, channel roles, binding, HEAD         *)
(*   owners           ──► agent grants, signed tags the agent carries       *)
(*   anyone           ──► pushes, approvals, check results, reads          *)
(*                              │                                          *)
(*                              ▼                                          *)
(*     Accept(p): one step judges a push against the authority in force    *)
(*     at that step, and refuses it with a receipt or reserves it          *)
(*     Publish(x): a later step moves all its refs and writes its receipt  *)
(*                                                                         *)
(* GitRefDB.tla says a push is applied only if it is authorized.  This     *)
(* module says what "authorized" means, and that it is judged when the    *)
(* push is accepted, never when it was received: a role, grant, rule or    *)
(* approval that is gone by then does not count.  Between acceptance and   *)
(* publication no authority write to the repo is applied.  Time passes, so *)
(* a grant can expire while a push it allowed publishes, and check results *)
(* can arrive; the receipt keeps the evidence counted at acceptance.       *)
(* MCGitGovernanceLand.tla checks that the relay's commit protocol is a    *)
(* refinement of this module.                                              *)
(*                                                                         *)
(* The module has two halves.  The first is the relay's evaluator, written *)
(* the way the relay computes it: the role it credits each key with, the   *)
(* effective rules, counted approvals, the latest trusted check.  The      *)
(* properties at the end restate each promise directly, without the       *)
(* evaluator's structure.  Faults switches on one evaluator defect at a    *)
(* time; docs/spec/check.sh requires each to violate the property named    *)
(* for it, so every property is shown to constrain what it talks about.   *)
(*                                                                         *)
(* Decisions this module makes, so they can be argued with:                *)
(*   - Authority comes only from the repo's bound channel, for every key.  *)
(*     Announcing a repo, owning the agent that announced it, or being     *)
(*     named in its event grants nothing.                                  *)
(*   - A key acts with its own role in the channel, agents included.  An  *)
(*     agent with a recorded owner may also act under a live grant from    *)
(*     that owner, at the lesser of the grant's level and the owner's      *)
(*     role, and takes the higher of the two routes.  A grant never        *)
(*     narrows, and it reaches the repo, not the channel.  An owner ban    *)
(*     ends both routes.  The agent's work is its owner's work.  A grant   *)
(*     is a tag the owner signs for the window it states and the agent     *)
(*     carries on each request.  Nothing ends it early but a ban of the    *)
(*     agent's key or its owner's.                                         *)
(*   - A push lands whole or not at all, and its commits stay in           *)
(*     quarantine, visible to no one, until it lands.  A repo has at most  *)
(*     one accepted push waiting to publish.                               *)
(*   - An approval names a repo, a ref, an exact commit, and the tip of    *)
(*     the ref when it was given.  Whether a later move of that tip makes  *)
(*     it stale is the rule's choice ("fresh-target"), not a universal one. *)
(*   - Approvals are counted at the decision, by reviewers who may act     *)
(*     then; one removed after approving no longer counts.                 *)
(*   - Code owners come from the ref being changed, so a change cannot     *)
(*     name its own reviewers, and owning code grants no access.           *)
(*   - A requirement the relay does not know denies.                       *)
(*   - A governance document changes only by a command that names the      *)
(*     document and the version it replaces, signed by a key that holds    *)
(*     the authority when the command is applied; so a captured command    *)
(*     cannot be replayed, here or onto another document.                  *)
(*   - Every decision, every read and every governance change is in the    *)
(*     record, and nothing leaves the record, not even when the channel    *)
(*     is archived.                                                        *)
(*                                                                         *)
(* Not modeled: commit signatures (the relay verifies none, so "own work"  *)
(* is every key that brought a commit to a ref of the repo), ref patterns  *)
(* (rules name refs), and secrets found after a push landed (removing     *)
(* them is an operator purge).                                            *)
(***************************************************************************)
EXTENDS Naturals, FiniteSets

CONSTANTS
    People,        \* keys without an owner attestation (NIP-OA)
    Agents,        \* keys that carry one
    Channels,      \* channels a repo can be bound to
    Repos,         \* repositories
    RepoKey,       \* RepoKey[r]: the key that announced repo r
    Branches, Tags,
    Commits,       \* commits; Parent gives the history
    Parent,        \* Parent[c]: c's parent, or NoCommit
    Secret,        \* commits whose content the door's scanner detects
    CodeOwners,    \* CodeOwners[c]: keys the code-owner file at c names ({} at NoCommit)
    CheckNames,    \* names of checks
    Labels,        \* protected label values
    Content,       \* Content[b]: what the governance body named b says
    NoCommit, None,
    \* The instance: its starting state and the choices each actor has.
    Init_,         \* record of the starting values of the state below
    GovChoices,    \* <<key, document, body>>: commands a key may sign
    RoleChanges,   \* <<channel, key, role>> a channel admin may apply
    Bans,          \* <<admin, key>>
    Registrations, \* <<agent, person>>: attestations an agent may present
    GrantChoices,  \* <<agent, repo, level>> an owner may grant
    Proposals,     \* <<key, repo, {<<ref, new commit>>, ...}>>: pushes
    Reviews,       \* <<key, repo, ref, commit>>: approvals
    Posts,         \* <<key, check, repo, commit, result>>: check results
    ReadChoices,   \* <<key, repo, door>>
    TagChoices,    \* <<key, repo, labels body>>: channel topic edits
    Archives,      \* repos whose channel may be archived
    Faults         \* seeded evaluator defects; {} is the design

Keys == People \cup Agents
Refs == Branches \cup Tags
CommitOrNone == Commits \cup {NoCommit}

RECURSIVE Ancestry(_)
Ancestry(c) == IF c = NoCommit THEN {} ELSE {c} \cup Ancestry(Parent[c])

ChanRoles == {"none", "guest", "member", "admin"}
Rank == [none |-> 0, guest |-> 1, member |-> 2, admin |-> 3]
MinRole(a, b) == IF Rank[a] <= Rank[b] THEN a ELSE b
MaxRole(a, b) == IF Rank[a] >= Rank[b] THEN a ELSE b

(***************************************************************************)
(* Governance documents.  Each is one relay-signed state, replaced whole   *)
(* by an admin command:                                                    *)
(*   <<"floor", None>>    rules for every repo, or for repos with a label  *)
(*   <<"trusted", None>>  <<check, key>>: who may report each check        *)
(*   <<"labels", r>>      protected labels of repo r                       *)
(*   <<"rules", r>>       rules repo r's admins add                        *)
(*   <<"bind", r>>        the channel whose roles are r's permissions      *)
(*   <<"head", r>>        r's default branch; the relay changes it by a    *)
(*                        land, not a document                             *)
(* A rule is [req, ref, scope, n, check]: scope is "all", a label, or     *)
(* "repo"; req is "admin-push", "no-force", "no-delete", "review" (n       *)
(* approvals), "fresh-target", "check" (the check named), "codeowners", or *)
(* any other value, which stands for a requirement this relay does not     *)
(* know, an old "require-patch" included.                                  *)
(***************************************************************************)
DocIds == {<<"floor", None>>, <<"trusted", None>>}
            \cup {<<t, r>> : t \in {"labels", "rules", "bind", "head"}, r \in Repos}

VARIABLES
    commRole,   \* commRole[k]: "none" (not admitted, or banned), "member", "admin"
    chanRole,   \* chanRole[ch][k]: k's role in channel ch
    owner,      \* owner[a]: the first owner recorded for agent a, or None
    grants,     \* live grants: <<agent, repo, level>>, level a channel role
    gov,        \* gov[d]: the name of governance document d's current body
    ver,        \* ver[d]: how many commands have replaced d
    sent,       \* every command ever signed; anyone may deliver one again
    govlog,     \* the record of governance: every command applied, in order
    refs,       \* refs[r][f]: the commit ref f of repo r names, or NoCommit
    visible,    \* visible[r]: commits a reader of r may be shown
    pushed,     \* <<key, repo, commit>>: the relay's record of who brought what
    brought,    \* the same, as it happened (history, for the properties)
    pending,    \* received pushes not yet judged
    reserved,   \* accepted pushes waiting to publish: [p, rec], one per repo
    approvals,  \* [by, repo, ref, head, base]
    posts,      \* check results: [seq, by, name, repo, head, result]
    receipts,   \* the record of decisions
    audit,      \* the record of reads
    act         \* the step just taken (history, for the properties)

govVars == <<commRole, chanRole, owner, grants, gov, ver, sent, govlog>>
gitVars == <<refs, visible, pushed, brought, pending, reserved, approvals, posts>>
vars    == <<govVars, gitVars, receipts, audit, act>>

Floor       == Content[gov[<<"floor", None>>]]
Trusted     == Content[gov[<<"trusted", None>>]]
LabelsOf(r) == Content[gov[<<"labels", r>>]]
RulesOf(r)  == Content[gov[<<"rules", r>>]]
ChanOf(r)   == Content[gov[<<"bind", r>>]]

\* Repo r has an accepted push waiting to publish.  An authority write that
\* could change its judgment waits until it publishes or is abandoned.
Fenced(r) == \E x \in reserved : x.p.repo = r
ChanFenced(ch) == \E r \in Repos : ChanOf(r) = ch /\ Fenced(r)

Init ==
    /\ commRole = Init_.commRole
    /\ chanRole = Init_.chanRole
    /\ owner = Init_.owner
    /\ grants = Init_.grants
    /\ gov = Init_.gov
    /\ ver = [d \in DocIds |-> 0]
    /\ sent = {}
    /\ govlog = {}
    /\ refs = Init_.refs
    /\ visible = Init_.visible
    /\ pushed = Init_.pushed
    /\ brought = Init_.pushed
    /\ pending = {}
    /\ reserved = {}
    /\ approvals = {}
    /\ posts = {}
    /\ receipts = {}
    /\ audit = {}
    /\ act = [kind |-> "init"]

-----------------------------------------------------------------------------
(***************************************************************************)
(* The relay's evaluator.                                                  *)
(***************************************************************************)

\* The person an act is credited to: an agent with a recorded owner counts
\* as its owner; any other key, an agent with none included, as itself.
Principal(k) ==
    IF k \in Agents /\ "agent-is-own-principal" \notin Faults
       /\ (owner[k] # None \/ "unowned-agent-unattributed" \in Faults)
    THEN owner[k] ELSE k

Owned(k) == k \in Agents /\ owner[k] # None

\* The role a live grant from k's owner credits k with in repo r: the
\* highest level among k's grants for r, capped by the owner's role there.
\* The relay sees one tag per request, so "highest" only matters here,
\* where an agent may hold several; check.sh does not guard the choice.
ViaGrant(k, r) ==
    LET ch == ChanOf(r)
        levels == {IF "grant-level-ignored" \in Faults THEN "admin" ELSE x[3] :
                     x \in {x \in grants : x[1] = k /\ (x[2] = r \/ "grant-any-repo" \in Faults)}}
        level == IF levels = {} THEN "none" ELSE CHOOSE l \in levels : \A l2 \in levels : Rank[l] >= Rank[l2]
    IN IF ~Owned(k) THEN "none"
       ELSE IF "grant-uncapped" \in Faults THEN level
       ELSE MinRole(level, chanRole[ch][owner[k]])

\* The role the relay credits k with in repo r: the higher of k's own role
\* in the repo's bound channel and, for an owned agent, what a live grant
\* gives it.  A ban of k, or of its recorded owner, credits nothing.
CreditedRole(k, r) ==
    LET ch == ChanOf(r) IN
    IF commRole[k] = "none" THEN "none"
    ELSE IF "announcer-is-owner" \in Faults /\ k = RepoKey[r] THEN "admin"
    ELSE IF "owner-borrows-agent-repo" \in Faults
            /\ RepoKey[r] \in Agents /\ owner[RepoKey[r]] = k THEN "admin"
    ELSE IF Owned(k) /\ commRole[owner[k]] = "none" /\ "owner-ban-ignored" \notin Faults THEN "none"
    ELSE IF Owned(k) /\ "agent-uses-owner" \in Faults THEN chanRole[ch][owner[k]]
    ELSE MaxRole(chanRole[ch][k], ViaGrant(k, r))

Holds(k, r, n) == Rank[CreditedRole(k, r)] >= Rank[n]

\* Who may replace a governance document.
MayCommand(k, d) ==
    LET r == d[2] IN
    CASE d[1] \in {"floor", "trusted"} ->
            commRole[k] = "admin"
              \/ ("repo-admin-floor" \in Faults /\ \E x \in Repos : Holds(k, x, "admin"))
      [] d[1] = "labels" ->
            commRole[k] = "admin" \/ ("repo-admin-labels" \in Faults /\ Holds(k, r, "admin"))
      [] d[1] = "rules" -> Holds(k, r, "admin")
      [] d[1] = "head" ->
            Holds(k, r, "admin") \/ ("head-by-maintainer" \in Faults /\ k = RepoKey[r])
      [] d[1] = "bind" ->
            Holds(k, r, "admin") \/ ("announcer-rebinds" \in Faults /\ k = RepoKey[r])

\* A rebind must also come from an admin of the channel it binds to.
MayBindTo(k, ch) ==
    \/ commRole[k] # "none" /\ chanRole[ch][k] = "admin"
    \/ "announcer-rebinds" \in Faults

FloorOn(r, f) ==
    {x \in Floor : /\ x.ref = f
                   /\ x.scope = "all" \/ x.scope \in LabelsOf(r)
                   /\ f \in Branches \/ "branches-only" \notin Faults}
RepoOn(r, f) == {x \in RulesOf(r) : x.ref = f}
EffectiveRules(r, f) ==
    IF "repo-overrides-floor" \in Faults /\ RepoOn(r, f) # {}
    THEN RepoOn(r, f) ELSE FloorOn(r, f) \cup RepoOn(r, f)

UpdateKind(c) ==
    IF c.old = NoCommit THEN "create"
    ELSE IF c.new = NoCommit THEN "delete"
    ELSE IF c.old \in Ancestry(c.new) THEN "ff" ELSE "nonff"

\* With no rule: members create refs and fast-forward branches; moving a
\* tag, rewriting history and deleting take an admin (git_perms.rs).
DefaultNeed(c) ==
    IF UpdateKind(c) = "create" \/ (UpdateKind(c) = "ff" /\ c.ref \in Branches)
    THEN "member" ELSE "admin"

Need(r, c) ==
    IF \E x \in EffectiveRules(r, c.ref) : x.req = "admin-push" THEN "admin" ELSE DefaultNeed(c)

\* The commits a push brings.  They sit in quarantine until it lands.
Incoming(p) == UNION {Ancestry(c.new) : c \in p.cmds} \ visible[p.repo]
Scanned(p) ==
    CASE "scan-tip-only" \in Faults -> Incoming(p) \cap {c.new : c \in p.cmds}
      [] "scan-governed-only" \in Faults ->
            Incoming(p) \cap UNION {Ancestry(c.new) : c \in {c \in p.cmds : EffectiveRules(p.repo, c.ref) # {}}}
      [] OTHER -> Incoming(p)

\* Whose work a ref change carries: every key that ever brought one of its
\* commits to a ref of the repo, and the pusher.
Brings(c) == Ancestry(c.new) \ Ancestry(c.old)
Authors(p, c) ==
    {Principal(e[1]) : e \in {e \in pushed : e[2] = p.repo /\ e[3] \in Brings(c)}}
      \cup (IF Brings(c) # {} THEN {Principal(p.by)} ELSE {})

IsCheckKey(k) == \E n \in CheckNames : <<n, k>> \in Trusted

FreshTarget(r, f) == \E x \in EffectiveRules(r, f) : x.req = "fresh-target"

\* An approval counts for a ref change only if it names the repo, the ref,
\* the exact new commit and, under a fresh-target rule, the current tip.
Bound(a, p, c) ==
    /\ a.repo = p.repo \/ "evidence-any-repo" \in Faults
    /\ a.ref = c.ref \/ "evidence-any-ref" \in Faults
    /\ a.head = c.new \/ ("stale-approval" \in Faults /\ a.head \in Ancestry(c.new))
    /\ FreshTarget(p.repo, c.ref) /\ "target-tip-ignored" \notin Faults => a.base = c.old

\* Who may fill a human approval: a key with no owner attestation that is
\* not a trusted check, may write the repo now, and did none of the work.
\* Distinct keys count as distinct people: the relay knows no more.
HumanSlot(a, p, c) ==
    /\ a.by \in People \/ "agent-approves" \in Faults
    /\ ~IsCheckKey(a.by) \/ "ci-as-human" \in Faults
    /\ IF "eligibility-at-approve" \in Faults THEN a.held ELSE Holds(a.by, p.repo, "member")
    /\ Principal(a.by) \notin Authors(p, c) \/ "self-approval" \in Faults

Counted(p, c) ==
    {a.by : a \in {a \in approvals : Bound(a, p, c) /\ HumanSlot(a, p, c)}}

TrustedPosts(n, r, x) ==
    {q \in posts : /\ q.name = n /\ q.repo = r /\ q.head = x
                   /\ \/ <<n, q.by>> \in Trusted
                      \/ "untrusted-check" \in Faults
                      \/ "trusted-for-any-check" \in Faults /\ IsCheckKey(q.by)}
CheckPasses(n, r, x) ==
    \E q \in TrustedPosts(n, r, x) :
        /\ q.result = "pass"
        /\ \A q2 \in TrustedPosts(n, r, x) : q2.seq <= q.seq \/ "first-pass-wins" \in Faults

\* Code owners come from the ref being changed, never from the change.
OwnersFor(c) == IF "codeowners-from-change" \in Faults THEN CodeOwners[c.new] ELSE CodeOwners[c.old]

Meets(x, p, c) ==
    CASE x.req = "admin-push"   -> TRUE   \* folded into Need
      [] x.req = "no-force"     -> UpdateKind(c) # "nonff"
      [] x.req = "no-delete"    -> UpdateKind(c) # "delete"
      [] x.req = "review"       -> Cardinality(Counted(p, c)) >= x.n
      [] x.req = "fresh-target" -> TRUE   \* folded into Bound
      [] x.req = "check"        -> CheckPasses(x.check, p.repo, c.new)
      [] x.req = "codeowners"   -> OwnersFor(c) = {} \/ Counted(p, c) \cap OwnersFor(c) # {}
      [] OTHER                  -> "skip-unknown" \in Faults

RefOk(p, c) ==
    /\ c.old = refs[p.repo][c.ref]
    /\ Holds(p.by, p.repo, Need(p.repo, c))
    /\ \A x \in EffectiveRules(p.repo, c.ref) : Meets(x, p, c)

\* A push lands whole or not at all.
Gate(p) ==
    /\ Scanned(p) \cap Secret = {}
    /\ IF "any-ref-decides" \in Faults THEN \E c \in p.cmds : RefOk(p, c)
                                       ELSE \A c \in p.cmds : RefOk(p, c)

\* What the relay acts on: the gate now, or, under the seeded error, the
\* gate as it stood when the push was received.
Verdict(p) == IF "authority-at-receive" \in Faults THEN p.pre ELSE Gate(p)

-----------------------------------------------------------------------------
(***************************************************************************)
(* Governance: commands, roles, agents.                                    *)
(*                                                                         *)
(* Any key may sign a command, and anyone holding a signed command may     *)
(* deliver it again, any number of times.  The relay applies a delivery    *)
(* only if the command names the version it replaces and its signer holds  *)
(* the authority now.                                                      *)
(***************************************************************************)

Sign(k, d, b) ==
    LET m == [by |-> k, doc |-> d, body |-> b, prev |-> ver[d],
              could |-> "authority-at-sign" \in Faults /\ MayCommand(k, d)]
    IN
    /\ ~\E x \in sent : x.by = k /\ x.doc = d /\ x.body = b
    /\ sent' = sent \cup {m}
    /\ act' = [kind |-> "sign"]
    /\ UNCHANGED <<commRole, chanRole, owner, grants, gov, ver, govlog, gitVars, receipts, audit>>

\* A delivery of command m applied to document d.  The command names its
\* document, so d is m.doc; the seeded error "command-unbound" lets it
\* land on another document of the same type.
Deliver(m, d) ==
    /\ d = m.doc \/ ("command-unbound" \in Faults /\ d[1] = m.doc[1])
    /\ m.prev = ver[d] \/ "freshness-window-only" \in Faults
    /\ IF "authority-at-sign" \in Faults THEN m.could ELSE MayCommand(m.by, d)
    /\ d[1] = "bind" => MayBindTo(m.by, Content[m.body])
    /\ IF d[2] = None /\ "fence-nominal" \notin Faults THEN reserved = {} ELSE ~Fenced(d[2])
    /\ gov' = [gov EXCEPT ![d] = m.body]
    /\ ver' = [ver EXCEPT ![d] = @ + 1]
    /\ govlog' = IF "unrecorded-governance" \in Faults THEN govlog
                 ELSE govlog \cup {[cmd |-> m, doc |-> d, ver |-> ver[d]]}
    /\ act' = [kind |-> "govern", cmd |-> m, doc |-> d]
    /\ UNCHANGED <<commRole, chanRole, owner, grants, sent, gitVars, receipts, audit>>

\* A channel admin adds, removes or re-roles a member: a key admitted, with
\* an admitted owner if it has one, whose own role in the channel is admin.
\* A grant reaches a repo, not its channel, so it never makes an agent a
\* channel admin.
ChannelAdmin(k, ch) ==
    /\ commRole[k] # "none"
    /\ Owned(k) => commRole[owner[k]] # "none"
    /\ \/ chanRole[ch][k] = "admin"
       \/ "grant-admins-channel" \in Faults /\ \E r \in Repos : ChanOf(r) = ch /\ Holds(k, r, "admin")

SetRole(k, x) ==
    /\ ChannelAdmin(k, x[1]) /\ ~ChanFenced(x[1])
    /\ chanRole[x[1]][x[2]] # x[3]
    /\ chanRole' = [chanRole EXCEPT ![x[1]][x[2]] = x[3]]
    /\ act' = [kind |-> "role", by |-> k, ch |-> x[1]]
    /\ UNCHANGED <<commRole, owner, grants, gov, ver, sent, govlog, gitVars, receipts, audit>>

\* A community admin bans a key: it holds no authority anywhere.  Banning
\* an agent is how a grant it carries is stopped before its window ends.
Ban(x) ==
    /\ commRole[x[1]] = "admin" /\ x[1] # x[2] /\ commRole[x[2]] # "none"
    /\ reserved = {} \/ "ban-unfenced" \in Faults
    /\ commRole' = [commRole EXCEPT ![x[2]] = "none"]
    /\ act' = [kind |-> "ban", by |-> x[1]]
    /\ UNCHANGED <<chanRole, owner, grants, gov, ver, sent, govlog, gitVars, receipts, audit>>

\* An agent presents an owner attestation.  The community keeps the first
\* owner it recorded; a later attestation naming someone else changes nothing.
Register(g) ==
    /\ owner[g[1]] # g[2]
    /\ owner[g[1]] = None \/ "owner-replaceable" \in Faults
    /\ reserved = {}
    /\ owner' = [owner EXCEPT ![g[1]] = g[2]]
    /\ act' = [kind |-> "register"]
    /\ UNCHANGED <<commRole, chanRole, grants, gov, ver, sent, govlog, gitVars, receipts, audit>>

\* An owner signs its agent a grant for a repo at a level.  It is a tag, not
\* a relay write, so the owner can sign one at any time and nothing waits
\* for it.  What it gives is capped by the owner's role, judged when the
\* agent uses it; it never takes away the agent's own role.
Grant(g) ==
    /\ g \notin grants /\ owner[g[1]] # None
    /\ grants' = grants \cup {g}
    /\ act' = [kind |-> "grant"]
    /\ UNCHANGED <<commRole, chanRole, owner, gov, ver, sent, govlog, gitVars, receipts, audit>>

\* Grants expire, any number at the same moment.  Expiry needs no one to
\* act and waits for nothing, so a push accepted under a grant can publish
\* after the grant lapsed.
Expire(G) ==
    /\ grants' = grants \ G
    /\ act' = [kind |-> "expire"]
    /\ UNCHANGED <<commRole, chanRole, owner, gov, ver, sent, govlog, gitVars, receipts, audit>>

\* A channel admin edits the channel's own topic labels.  They describe the
\* channel; they are not the repo's protected labels.
ChannelTag(k, r, b) ==
    /\ Holds(k, r, "admin")
    /\ gov' = IF "channel-tags-protect" \in Faults
              THEN [gov EXCEPT ![<<"labels", r>>] = b] ELSE gov
    /\ act' = [kind |-> "channel-tag"]
    /\ UNCHANGED <<commRole, chanRole, owner, grants, ver, sent, govlog, gitVars, receipts, audit>>

\* The repo's channel is archived: its roles end.  The record outlives it.
Archive(r) ==
    /\ \E k \in Keys : chanRole[ChanOf(r)][k] # "none"
    /\ ~ChanFenced(ChanOf(r))
    /\ chanRole' = [chanRole EXCEPT ![ChanOf(r)] = [k \in Keys |-> "none"]]
    /\ receipts' = IF "archive-drops-record" \in Faults
                   THEN {x \in receipts : x.repo # r} ELSE receipts
    /\ act' = [kind |-> "archive"]
    /\ UNCHANGED <<commRole, owner, grants, gov, ver, sent, govlog, gitVars, audit>>

-----------------------------------------------------------------------------
(***************************************************************************)
(* Changes: pushes, evidence, the decision, reads.                         *)
(***************************************************************************)

\* Pushes received and not yet published or abandoned.
Waiting == pending \cup {x.p : x \in reserved}

\* A key pushes.  The commits go to quarantine and the ref changes wait for
\* the decision.  One push per key at a time.  pre is the verdict as it
\* stood on receipt; only the seeded error "authority-at-receive" uses it.
Propose(e) ==
    LET cmds == {[ref |-> x[1], old |-> refs[e[2]][x[1]], new |-> x[2]] : x \in e[3]}
        p0 == [by |-> e[1], repo |-> e[2], cmds |-> cmds]
    IN
    /\ ~\E q \in Waiting : q.by = e[1]
    /\ \A c \in cmds : c.old # c.new
    /\ pending' = pending \cup {[by |-> e[1], repo |-> e[2], cmds |-> cmds,
                                 pre |-> "authority-at-receive" \in Faults /\ Gate(p0)]}
    /\ visible' = IF "visible-before-scan" \in Faults
                  THEN [visible EXCEPT ![e[2]] = @ \cup Incoming(p0)] ELSE visible
    /\ act' = [kind |-> "push"]
    /\ UNCHANGED <<govVars, refs, pushed, brought, reserved, approvals, posts, receipts, audit>>

\* A review approves a commit for a ref.  It records the ref's tip as the
\* reviewer saw it, and whether the reviewer could write the repo then
\* (only the seeded error "eligibility-at-approve" reads that).
Approve(e) ==
    /\ e[4] \in visible[e[2]]
    /\ approvals' = approvals \cup
         {[by |-> e[1], repo |-> e[2], ref |-> e[3], head |-> e[4],
           base |-> refs[e[2]][e[3]],
           held |-> "eligibility-at-approve" \in Faults /\ Holds(e[1], e[2], "member")]}
    /\ act' = [kind |-> "approve"]
    /\ UNCHANGED <<govVars, refs, visible, pushed, brought, pending, reserved, posts, receipts, audit>>

\* Anyone can post a check result.  Only a trusted one counts.
Post(e) ==
    /\ e[4] \in visible[e[3]]
    /\ ~\E q \in posts : <<q.by, q.name, q.repo, q.head, q.result>> = e
    /\ posts' = posts \cup {[seq |-> Cardinality(posts), by |-> e[1], name |-> e[2],
                             repo |-> e[3], head |-> e[4], result |-> e[5]]}
    /\ act' = [kind |-> "post"]
    /\ UNCHANGED <<govVars, refs, visible, pushed, brought, pending, reserved, approvals, receipts, audit>>

\* The receipt names who acted and for whom, every ref change, the verdict,
\* the rules in force for each ref, and the approvals and check results
\* counted for each.  Decision is the receipt without its place in the
\* record; Stamp gives it one.
Decision(p, ok) ==
    [by |-> p.by, for |-> Principal(p.by), repo |-> p.repo, cmds |-> p.cmds, ok |-> ok,
     rules |-> {<<c.ref, EffectiveRules(p.repo, c.ref)>> : c \in p.cmds},
     evidence |-> IF "receipt-omits-evidence" \in Faults THEN {}
                  ELSE UNION {{a \in approvals : Bound(a, p, c) /\ HumanSlot(a, p, c)} : c \in p.cmds},
     checks   |-> IF "receipt-omits-checks" \in Faults THEN {}
                  ELSE UNION {UNION {{q \in TrustedPosts(x.check, p.repo, c.new) :
                                         /\ q.result = "pass"
                                         /\ \A q2 \in TrustedPosts(x.check, p.repo, c.new) :
                                               q2.seq <= q.seq} :
                                       x \in {x \in EffectiveRules(p.repo, c.ref) : x.req = "check"}} :
                              c \in p.cmds}]

Stamp(x, n) ==
    [seq |-> n, by |-> x.by, for |-> x.for, repo |-> x.repo, cmds |-> x.cmds, ok |-> x.ok,
     rules |-> x.rules, evidence |-> x.evidence, checks |-> x.checks]

Receipt(p, ok) == Stamp(Decision(p, ok), Cardinality(receipts))

\* The decision, in two steps.  Accept judges the push against the roles,
\* grants, rules and evidence in force now.  A refusal discards its commits
\* and is receipted at once.  An acceptance reserves the push with the
\* receipt it will carry, and no other push of the repo is judged until it
\* publishes or is abandoned.  Publish moves every ref the push names,
\* releases its commits and writes the reserved receipt.  Abort abandons
\* the reservation (the relay died before it committed) and leaves no trace.
\* Drop is a push the relay never judged (it died first): no one is told
\* anything, and nothing is recorded.
Accept(p) ==
    /\ Verdict(p) => ~Fenced(p.repo) \/ "unfenced-accept" \in Faults
    /\ pending' = pending \ {p}
    /\ IF Verdict(p)
       THEN /\ reserved' = reserved \cup {[p |-> p, rec |-> Decision(p, TRUE)]}
            /\ receipts' = IF "receipt-at-accept" \in Faults
                           THEN receipts \cup {Receipt(p, TRUE)} ELSE receipts
            /\ UNCHANGED visible
       ELSE /\ visible' = IF "release-on-refusal" \in Faults
                          THEN [visible EXCEPT ![p.repo] = @ \cup Incoming(p)] ELSE visible
            /\ receipts' = IF "no-receipt-on-refusal" \in Faults
                           THEN receipts ELSE receipts \cup {Receipt(p, FALSE)}
            /\ UNCHANGED reserved
    /\ act' = [kind |-> "accept", p |-> p, ok |-> Verdict(p)]
    /\ UNCHANGED <<govVars, refs, pushed, brought, approvals, posts, audit>>

Publish(x) ==
    LET p == x.p IN
    /\ refs' = [refs EXCEPT ![p.repo] =
                  [f \in Refs |-> IF \E c \in p.cmds : c.ref = f
                                  THEN (CHOOSE c \in p.cmds : c.ref = f).new
                                  ELSE refs[p.repo][f]]]
    /\ visible' = [visible EXCEPT ![p.repo] = @ \cup Incoming(p)]
    /\ pushed' = pushed \cup {<<p.by, p.repo, y>> : y \in
                     IF "pusher-of-new-only" \in Faults THEN Incoming(p)
                     ELSE UNION {Brings(c) : c \in p.cmds}}
    /\ brought' = brought \cup {<<p.by, p.repo, y>> : y \in UNION {Brings(c) : c \in p.cmds}}
    /\ reserved' = reserved \ {x}
    /\ receipts' = CASE "receipt-at-accept" \in Faults -> receipts
                     [] "receipt-at-publish" \in Faults -> receipts \cup {Receipt(p, TRUE)}
                     [] OTHER -> receipts \cup {Stamp(x.rec, Cardinality(receipts))}
    /\ act' = [kind |-> "publish", p |-> p]
    /\ UNCHANGED <<govVars, pending, approvals, posts, audit>>

Abort(x) ==
    /\ reserved' = reserved \ {x}
    /\ receipts' = IF "abort-receipted" \in Faults
                   THEN receipts \cup {Stamp(x.rec, Cardinality(receipts))} ELSE receipts
    /\ act' = [kind |-> "abort", p |-> x.p]
    /\ UNCHANGED <<govVars, refs, visible, pushed, brought, pending, approvals, posts, audit>>
Drop(p) ==
    /\ pending' = pending \ {p}
    /\ act' = [kind |-> "drop", p |-> p]
    /\ UNCHANGED <<govVars, refs, visible, pushed, brought, reserved, approvals, posts,
                   receipts, audit>>

\* Doors that show a repo's content: a fetch, the evidence views (review,
\* diff, check pages), and the relay-signed governance state of the repo.
Doors == {"fetch", "evidence", "state"}

MayRead(k, r, door) ==
    CASE door = "evidence" /\ "evidence-for-community" \in Faults -> commRole[k] # "none"
      [] door = "state" /\ "state-for-community" \in Faults       -> commRole[k] # "none"
      [] OTHER -> Holds(k, r, "guest")

Shown(r, door) ==
    IF door = "evidence" /\ "evidence-shows-quarantine" \in Faults
    THEN visible[r] \cup UNION {Incoming(p) : p \in {p \in pending : p.repo = r}}
    ELSE visible[r]

Read(k, r, door) ==
    /\ MayRead(k, r, door)
    /\ audit' = IF "unrecorded-read" \in Faults THEN audit
                ELSE audit \cup {[seq |-> Cardinality(audit), by |-> k, repo |-> r, door |-> door]}
    /\ act' = [kind |-> "read", by |-> k, repo |-> r, door |-> door, shown |-> Shown(r, door)]
    /\ UNCHANGED <<govVars, gitVars, receipts>>

Next ==
    \/ \E x \in GovChoices : Sign(x[1], x[2], x[3])
    \/ \E m \in sent, d \in DocIds : Deliver(m, d)
    \/ \E x \in RoleChanges, k \in Keys : SetRole(k, x)
    \/ \E x \in Bans : Ban(x)
    \/ \E g \in Registrations : Register(g)
    \/ \E g \in GrantChoices : Grant(g)
    \/ \E G \in SUBSET grants : G # {} /\ Expire(G)
    \/ \E x \in TagChoices : ChannelTag(x[1], x[2], x[3])
    \/ \E r \in Archives : Archive(r)
    \/ \E e \in Proposals : Propose(e)
    \/ \E e \in Reviews : Approve(e)
    \/ \E e \in Posts : Post(e)
    \/ \E x \in ReadChoices : Read(x[1], x[2], x[3])
    \/ \E p \in pending : Accept(p) \/ Drop(p)
    \/ \E x \in reserved : Publish(x) \/ Abort(x)

Spec == Init /\ [][Next]_vars

-----------------------------------------------------------------------------
(***************************************************************************)
(* The promises, stated without the evaluator.                             *)
(***************************************************************************)

TypeOK ==
    /\ commRole \in [Keys -> {"none", "member", "admin"}]
    /\ chanRole \in [Channels -> [Keys -> ChanRoles]]
    /\ owner \in [Agents -> People \cup {None}]
    /\ grants \subseteq Agents \X Repos \X (ChanRoles \ {"none"})
    /\ refs \in [Repos -> [Refs -> CommitOrNone]]
    /\ \A r \in Repos : visible[r] \subseteq Commits /\ ChanOf(r) \in Channels
    /\ \A x \in receipts : x.for \in Keys

\* k may act at level n in repo r now.  Authority comes only from the
\* channel r is bound to, by one of two routes: k's own role there, or,
\* for an agent with a recorded owner, a live grant for r at level n or
\* above from an owner who holds level n there himself.  Neither k nor its
\* owner is banned.
MayAct(k, r, n) ==
    LET ch == ChanOf(r) IN
    /\ commRole[k] # "none"
    /\ k \in Agents /\ owner[k] # None => commRole[owner[k]] # "none"
    /\ \/ Rank[chanRole[ch][k]] >= Rank[n]
       \/ /\ k \in Agents /\ owner[k] # None
          /\ \E x \in grants : x[1] = k /\ x[2] = r /\ Rank[x[3]] >= Rank[n]
          /\ Rank[chanRole[ch][owner[k]]] >= Rank[n]

Person(k) == IF k \in Agents /\ owner[k] # None THEN owner[k] ELSE k

\* Every rule in force for ref f of repo r: the floor's rules for every
\* repo and for r's protected labels, and r's own rules.  None replaces
\* another.
Governing(r, f) ==
    {x \in Floor : x.ref = f /\ (x.scope = "all" \/ x.scope \in LabelsOf(r))}
        \cup {x \in RulesOf(r) : x.ref = f}

Kind(c) ==
    IF c.old = NoCommit THEN "create" ELSE IF c.new = NoCommit THEN "delete"
    ELSE IF c.old \in Ancestry(c.new) THEN "ff" ELSE "nonff"

\* Commits a push brings that the repo has not shown anyone.
New(p) == UNION {Ancestry(c.new) : c \in p.cmds} \ visible[p.repo]

\* The people whose work a ref change carries.  An agent's is its owner's.
WorkOf(p, c) ==
    LET b == Ancestry(c.new) \ Ancestry(c.old) IN
    {Person(e[1]) : e \in {e \in brought : e[2] = p.repo /\ e[3] \in b}}
        \cup (IF b # {} THEN {Person(p.by)} ELSE {})

\* h may fill a human approval for this ref change now.
Reviewer(h, p, c) ==
    /\ \E a \in approvals :
          /\ a.by = h /\ a.repo = p.repo /\ a.ref = c.ref /\ a.head = c.new
          /\ (\E x \in Governing(p.repo, c.ref) : x.req = "fresh-target") => a.base = c.old
    /\ h \in People
    /\ \A n \in CheckNames : <<n, h>> \notin Trusted
    /\ MayAct(h, p.repo, "member")
    /\ h \notin WorkOf(p, c)

\* The newest result a trusted reporter posted for exactly this commit.
LatestTrustedPass(n, r, x) ==
    LET mine == {q \in posts : q.name = n /\ q.repo = r /\ q.head = x /\ <<n, q.by>> \in Trusted}
    IN \E q \in mine : q.result = "pass" /\ ~\E q2 \in mine : q2.seq > q.seq

Satisfied(x, p, c) ==
    CASE x.req = "admin-push"   -> MayAct(p.by, p.repo, "admin")
      [] x.req = "no-force"     -> Kind(c) # "nonff"
      [] x.req = "no-delete"    -> Kind(c) # "delete"
      [] x.req = "review"       -> Cardinality({h \in People : Reviewer(h, p, c)}) >= x.n
      [] x.req = "fresh-target" -> TRUE   \* a condition on approvals, above
      [] x.req = "check"        -> LatestTrustedPass(x.check, p.repo, c.new)
      [] x.req = "codeowners"   -> CodeOwners[c.old] # {} =>
                                     \E h \in CodeOwners[c.old] : Reviewer(h, p, c)
      [] OTHER                  -> FALSE

\* Without rules: members create refs and fast-forward branches; anything
\* else takes an admin.
Baseline(c) ==
    IF Kind(c) = "create" \/ (Kind(c) = "ff" /\ c.ref \in Branches) THEN "member" ELSE "admin"

Accepted(p) == act'.kind = "accept" /\ act'.p = p /\ act'.ok

\* Every push accepted was allowed, ref by ref, by the authority in force
\* at the step it was accepted: the pusher's role, an agent's grant and
\* owner, every rule that governs the ref, and evidence about exactly this
\* change from reviewers who may act now.
LandsUnderCurrentAuthority ==
    [][\A p \in pending : Accepted(p) =>
          \A c \in p.cmds :
             /\ c.old = refs[p.repo][c.ref]
             /\ MayAct(p.by, p.repo, Baseline(c))
             /\ \A x \in Governing(p.repo, c.ref) : Satisfied(x, p, c)]_vars

\* Refs move only when an accepted push publishes, all of its refs
\* together, from the values it was judged against to what it asked for.
RefsMoveOnlyByLanding ==
    [][\A r \in Repos : refs'[r] # refs[r] =>
          \E x \in reserved : LET p == x.p IN
                             /\ act'.kind = "publish" /\ act'.p = p /\ p.repo = r
                             /\ \A c \in p.cmds : c.old = refs[r][c.ref]
                             /\ \A f \in Refs :
                                  refs'[r][f] = IF \E c \in p.cmds : c.ref = f
                                                THEN (CHOOSE c \in p.cmds : c.ref = f).new
                                                ELSE refs[r][f]]_vars

\* What a repo has ever shown: its starting commits and what landed since.
Landed(r) == Init_.visible[r] \cup {e[3] : e \in {e \in brought : e[2] = r}}

\* Nothing a push brings is shown anywhere, through any door, until it
\* lands; and nothing that carries a detected secret ever lands.
QuarantineHolds ==
    /\ \A r \in Repos : visible[r] \subseteq Landed(r)
    /\ act.kind = "read" => act.shown \subseteq Landed(act.repo)
SecretsNeverShown ==
    /\ \A r \in Repos : visible[r] \cap Secret = {}
    /\ act.kind = "read" => act.shown \cap Secret = {}

RefsNameShownCommits ==
    \A r \in Repos, f \in Refs : refs[r][f] # NoCommit => refs[r][f] \in visible[r]

\* A ref that no-force and no-delete govern keeps its history: it always
\* names a descendant of what it named.
GuardedHistoryKept ==
    [][\A r \in Repos, f \in Refs :
          /\ \E x \in Governing(r, f) : x.req = "no-force"
          /\ \E x \in Governing(r, f) : x.req = "no-delete"
          /\ refs[r][f] # NoCommit
          => refs'[r][f] # NoCommit /\ refs[r][f] \in Ancestry(refs'[r][f])]_vars

\* Every read is by a key that may read the repo now, and is recorded.
ReadsAreGovernedAndRecorded ==
    [][act'.kind = "read" =>
          /\ MayAct(act'.by, act'.repo, "guest")
          /\ \E x \in audit' \ audit : x.by = act'.by /\ x.repo = act'.repo /\ x.door = act'.door]_vars

\* A governance document changes only by a command for that document,
\* written against its current version: a command applies at most once and
\* only where it was meant to.
CommandsApplyOnceWhereMeant ==
    [][act'.kind = "govern" => act'.doc = act'.cmd.doc /\ act'.cmd.prev = ver[act'.doc]]_vars

\* ... and only if its signer holds that authority when it applies.  A
\* rebind needs an admin of both channels.
GovernanceByCurrentAuthority ==
    [][act'.kind = "govern" =>
          LET k == act'.cmd.by  d == act'.doc IN
          /\ d[1] \in {"floor", "trusted", "labels"} => commRole[k] = "admin"
          /\ d[1] \in {"rules", "head", "bind"} => MayAct(k, d[2], "admin")
          /\ d[1] = "bind" => commRole[k] # "none" /\ chanRole[Content[act'.cmd.body]][k] = "admin"]_vars

\* Documents change only by a governance command; nothing else touches them.
GovernedDocsOnlyByCommand ==
    [][gov' # gov => act'.kind = "govern"]_vars

\* An agent's recorded owner never changes.
OwnerPinned == [][\A a \in Agents : owner[a] # None => owner'[a] = owner[a]]_vars

\* The receipt a decision of push p must carry, judged in this state: who
\* acted, for whom, every ref change, the verdict, the rules in force for
\* each ref, and the approvals and check results that counted.
IsReceiptOf(x, p, ok) ==
    /\ x.by = p.by /\ x.for = Person(p.by) /\ x.repo = p.repo
    /\ x.cmds = p.cmds /\ x.ok = ok
    /\ x.rules = {<<c.ref, Governing(p.repo, c.ref)>> : c \in p.cmds}
    /\ x.evidence =
          UNION {{a \in approvals :
                    /\ a.repo = p.repo /\ a.ref = c.ref /\ a.head = c.new
                    /\ Reviewer(a.by, p, c)
                    /\ (\E y \in Governing(p.repo, c.ref) : y.req = "fresh-target")
                          => a.base = c.old} :
                 c \in p.cmds}
    /\ x.checks =
          UNION {{q \in posts :
                    /\ q.repo = p.repo /\ q.head = c.new /\ q.result = "pass"
                    /\ <<q.name, q.by>> \in Trusted
                    /\ \E y \in Governing(p.repo, c.ref) : y.req = "check" /\ y.check = q.name
                    /\ ~\E q2 \in posts : /\ q2.name = q.name /\ q2.repo = q.repo
                                          /\ q2.head = q.head /\ <<q.name, q2.by>> \in Trusted
                                          /\ q2.seq > q.seq} :
                 c \in p.cmds}

Unnumbered(x) ==
    [by |-> x.by, for |-> x.for, repo |-> x.repo, cmds |-> x.cmds, ok |-> x.ok,
     rules |-> x.rules, evidence |-> x.evidence, checks |-> x.checks]

\* Every decision is in the record exactly once, with what it was judged
\* on: a refusal when it is refused, an acceptance when it publishes, with
\* the evidence it was accepted on even if more arrived since.  A push that
\* is accepted and then abandoned leaves no receipt, and no other step
\* writes one.
ReceiptsMatchDecisions ==
    [][LET new == receipts' \ receipts IN
       CASE act'.kind = "accept" /\ ~act'.ok ->
              \E x \in new : new = {x} /\ IsReceiptOf(x, act'.p, FALSE)
         [] act'.kind = "accept" ->
              /\ new = {}
              /\ \E y \in reserved' : y.p = act'.p /\ IsReceiptOf(y.rec, act'.p, TRUE)
         [] act'.kind = "publish" ->
              \E x \in new, y \in reserved :
                  new = {x} /\ y.p = act'.p /\ Unnumbered(x) = y.rec
         [] OTHER -> new = {}]_vars

\* While a push of repo r waits to publish, nothing its judgment rests on
\* changes: no community role, no recorded owner, no role in r's channel,
\* no governance document of the community or of r, and no grant for r
\* ends except by expiry.  A new grant can be signed, which takes nothing
\* away.  Other repos stay writable.
ReservationsFenced ==
    [][\A r \in Repos : Fenced(r) =>
          /\ commRole' = commRole /\ owner' = owner
          /\ chanRole'[ChanOf(r)] = chanRole[ChanOf(r)]
          /\ \A d \in DocIds : d[2] \in {None, r} => gov'[d] = gov[d]
          /\ act'.kind # "expire" => {x \in grants : x[2] = r} \subseteq grants']_vars

\* Every change of who may act in a channel is made by an admin of that
\* channel in its own right, not through a grant, and not while it or its
\* owner is banned.
RoleChangesByCurrentAuthority ==
    [][act'.kind = "role" =>
          LET k == act'.by  ch == act'.ch IN
          /\ commRole[k] # "none" /\ chanRole[ch][k] = "admin"
          /\ k \in Agents /\ owner[k] # None => commRole[owner[k]] # "none"]_vars

EveryGovernanceChangeRecorded ==
    [][act'.kind = "govern" =>
          [cmd |-> act'.cmd, doc |-> act'.doc, ver |-> ver[act'.doc]] \in govlog']_vars

\* Nothing leaves the record.
RecordOnlyGrows ==
    [][receipts \subseteq receipts' /\ audit \subseteq audit' /\ govlog \subseteq govlog']_vars

-----------------------------------------------------------------------------
\* Coverage: each must be violated by the design, which shows the model
\* reaches the behavior.
NeverLandsReviewed ==
    ~\E x \in receipts : x.ok /\ x.evidence # {} /\ \E y \in x.rules : \E z \in y[2] : z.req = "review"
NeverLandsChecked ==
    ~\E x \in receipts : x.ok /\ x.checks # {} /\ \E y \in x.rules : \E z \in y[2] : z.req = "check"
NeverLandsOwned ==
    ~\E x \in receipts : x.ok /\ x.evidence # {} /\ \E y \in x.rules : \E z \in y[2] : z.req = "codeowners"
NeverLandsByAgent == ~\E x \in receipts : x.ok /\ x.by \in Agents
\* An owned agent is accepted on its own channel role, holding no grant for
\* the repo; and one is accepted with no role in the channel at all.
NeverLandsByMemberAgentWithoutGrant ==
    [][\A p \in pending : Accepted(p) /\ p.by \in Agents /\ owner[p.by] # None =>
          \E x \in grants : x[1] = p.by /\ x[2] = p.repo]_vars
NeverLandsByGrantOnlyAgent ==
    [][\A p \in pending : Accepted(p) /\ p.by \in Agents =>
          chanRole[ChanOf(p.repo)][p.by] # "none"]_vars
\* A grant never narrows: an agent is accepted on its own role while every
\* grant it holds for the repo is below what the push needs, its owner
\* still in the channel so the grant still credits something.
NeverLandsOverLowerGrant ==
    [][\A p \in pending : Accepted(p) /\ p.by \in Agents /\ owner[p.by] # None
          /\ chanRole[ChanOf(p.repo)][owner[p.by]] # "none"
          /\ (\E x \in grants : x[1] = p.by /\ x[2] = p.repo) =>
          \E x \in grants : /\ x[1] = p.by /\ x[2] = p.repo
                           /\ \A c \in p.cmds : Rank[x[3]] >= Rank[Baseline(c)]]_vars
\* A member agent keeps its own role when its owner leaves the channel.
NeverLandsWithOwnerOutside ==
    [][\A p \in pending : Accepted(p) /\ p.by \in Agents /\ owner[p.by] # None =>
          chanRole[ChanOf(p.repo)][owner[p.by]] # "none"]_vars
\* An agent with no recorded owner acts on its own role.
NeverLandsByUnownedAgent ==
    [][\A p \in pending : Accepted(p) /\ p.by \in Agents => owner[p.by] # None]_vars
\* A grant reaches the repo of a channel the agent is not in, reads included.
NeverReadsByGrantOnlyAgent ==
    [][act'.kind = "read" /\ act'.by \in Agents =>
          chanRole[ChanOf(act'.repo)][act'.by] # "none"]_vars
NeverRebinds == ~\E x \in govlog : x.doc[1] = "bind"
\* A push accepted and then abandoned.
NeverAborts == [][act'.kind # "abort"]_vars

\* A push accepted under a grant publishes after the grant expired, when its
\* agent may no longer even read the repo.
NeverPublishesAfterExpiry ==
    [][act'.kind = "publish" => MayAct(act'.p.by, act'.p.repo, "guest")]_vars

\* An admin changes another repo's rules while a push of r1 waits to publish.
NeverGovernsWhileReserved == [][act'.kind = "govern" => reserved = {}]_vars

\* An approval given before the target moved still counts without a
\* fresh-target rule.
NeverCountsOldBase ==
    ~\E x \in receipts : x.ok /\ \E a \in x.evidence, c \in x.cmds : a.ref = c.ref /\ a.base # c.old
=============================================================================
