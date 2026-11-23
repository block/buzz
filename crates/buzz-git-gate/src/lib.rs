//! The governed Git decision as one pure function that Cargo runs and Verus
//! checks.
//!
//! `decide` is the relay's evaluator for one push: the role it credits each
//! key with, the rules in force for each ref, the approvals it counts and the
//! newest trusted check. Its `ensures` clause states that it allows a push
//! exactly when `lands_ok` holds, and `lands_ok` restates
//! `LandsUnderCurrentAuthority` from `docs/spec/GitGovernance.tla` directly,
//! without the evaluator's structure. Both directions are proved, so the
//! evaluator can neither allow a push the property forbids nor deny one it
//! permits.
//!
//! Cargo compiles the executable bodies inside `verus!` with the proof and
//! specification code erased; `scripts/verify-git-gate.sh` checks the same
//! bodies with it present. There is no second executable model.
//!
//! The caller resolves every input from the state in force at the decision:
//! roles from the repo's bound channel only, the owner's current standing,
//! live grants, the floor for the repo's protected labels, the repo's rules,
//! approvals and check results, and code owners from the ref being changed.
//! A fact the input has no field for cannot affect the verdict; that is how
//! announcing a repo or owning its announcer is kept from granting anything.
use vstd::prelude::*;

verus! {

/// A key's role in the repo's bound channel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role { None, Guest, Member, Admin }

pub open spec fn rank(r: Role) -> nat {
    match r { Role::None => 0, Role::Guest => 1, Role::Member => 2, Role::Admin => 3 }
}

fn rank_of(r: Role) -> (out: u8)
    ensures out as nat == rank(r),
{
    match r { Role::None => 0, Role::Guest => 1, Role::Member => 2, Role::Admin => 3 }
}

/// An agent's first recorded owner, with that owner's standing now.
#[derive(Clone, Copy, Debug)]
pub struct Owner { pub key: u64, pub admitted: bool, pub role: Role }

/// Whether a key carries an owner attestation, and whose.
#[derive(Clone, Copy, Debug)]
pub enum Who { Person, UnownedAgent, Agent(Owner) }

/// What the relay knows about a key now.
#[derive(Clone, Copy, Debug)]
pub struct KeyFacts {
    pub key: u64,
    /// Admitted to the community and not banned.
    pub admitted: bool,
    /// Role in the repo's bound channel.
    pub role: Role,
    pub who: Who,
    /// For an agent: a live grant from its owner for this repo.
    pub granted: bool,
    /// The key is trusted to report some check.
    pub check_key: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Req { AdminPush, NoForce, NoDelete, Review, FreshTarget, Check, CodeOwners, Unknown }

#[derive(Clone, Copy, Debug)]
pub enum Scope { All, Label(u64) }

/// A rule naming a ref. `n` is the approvals a review needs; `check` names
/// the check a check rule needs. `Unknown` is any requirement this relay
/// does not implement.
#[derive(Clone, Copy, Debug)]
pub struct Rule { pub req: Req, pub git_ref: u64, pub scope: Scope, pub n: u64, pub check: u64 }

/// A review: the repo, ref and commit it approves, and the ref's tip when
/// it was given.
#[derive(Clone, Copy, Debug)]
pub struct Approval { pub by: KeyFacts, pub repo: u64, pub git_ref: u64, pub head: u64, pub base: u64 }

/// A check result. `seq` orders results; a later one supersedes.
#[derive(Clone, Copy, Debug)]
pub struct Post { pub by: u64, pub check: u64, pub head: u64, pub pass: bool, pub seq: u64 }

#[derive(Clone, Copy, Debug)]
pub struct Reporter { pub check: u64, pub key: u64 }

/// Whether the push's quarantined commits were scanned and found clean.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scan { Clean, Dirty, Unknown }

/// The commit id that means "no commit": a ref being created or deleted.
pub const NO_COMMIT: u64 = 0;

/// One ref change of a push.
#[derive(Clone, Debug)]
pub struct Cmd {
    pub git_ref: u64,
    pub is_branch: bool,
    /// The tip the push expects to replace, and the tip the ref names now.
    pub old: u64,
    pub current: u64,
    pub new: u64,
    /// `old` is an ancestor of `new`.
    pub fast_forward: bool,
    /// The change brings commits to the ref: some commit reachable from
    /// `new` is not reachable from `old`, whether or not the repo has shown
    /// it before. Bringing one makes the pusher an author of the change.
    pub brings: bool,
    /// The people who ever brought to any ref of the repo a commit this
    /// change brings (an agent's work is its owner's).
    pub authors: Vec<u64>,
    /// Code owners named by the owners file at `old`.
    pub owners: Vec<u64>,
}

pub struct Input {
    pub repo: u64,
    pub pusher: KeyFacts,
    pub scan: Scan,
    pub cmds: Vec<Cmd>,
    /// The community floor, and the repo's protected labels.
    pub floor: Vec<Rule>,
    pub labels: Vec<u64>,
    /// The rules the repo's admins add. Their scope is ignored.
    pub repo_rules: Vec<Rule>,
    pub trusted: Vec<Reporter>,
    pub approvals: Vec<Approval>,
    pub posts: Vec<Post>,
}

// ---------------------------------------------------------------------------
// The property, stated without the evaluator.
// ---------------------------------------------------------------------------

/// k may act at level n now. Authority comes only from the bound channel; an
/// agent also needs a live grant and an owner who may act at level n.
pub open spec fn may_act(k: KeyFacts, n: Role) -> bool {
    k.admitted && rank(k.role) >= rank(n) && match k.who {
        Who::Person => true,
        Who::UnownedAgent => false,
        Who::Agent(o) => k.granted && o.admitted && rank(o.role) >= rank(n),
    }
}

pub open spec fn person(k: KeyFacts) -> u64 {
    match k.who { Who::Agent(o) => o.key, _ => k.key }
}

pub open spec fn has_label(i: Input, l: u64) -> bool {
    exists|j: int| 0 <= j < i.labels.len() && i.labels[j] == l
}

pub open spec fn floor_governs(i: Input, c: Cmd, x: Rule) -> bool {
    x.git_ref == c.git_ref && match x.scope { Scope::All => true, Scope::Label(l) => has_label(i, l) }
}

pub open spec fn fresh_target(i: Input, c: Cmd) -> bool {
    (exists|j: int| 0 <= j < i.floor.len() && floor_governs(i, c, i.floor[j]) && i.floor[j].req == Req::FreshTarget)
    || (exists|j: int| 0 <= j < i.repo_rules.len() && i.repo_rules[j].git_ref == c.git_ref
           && i.repo_rules[j].req == Req::FreshTarget)
}

pub open spec fn work_of(i: Input, c: Cmd, h: u64) -> bool {
    (exists|j: int| 0 <= j < c.authors.len() && c.authors[j] == h)
    || (c.brings && person(i.pusher) == h)
}

/// The approval names exactly this change and fills a human slot now.
pub open spec fn reviews(i: Input, c: Cmd, a: Approval) -> bool {
    a.repo == i.repo && a.git_ref == c.git_ref && a.head == c.new
    && (fresh_target(i, c) ==> a.base == c.old)
    && a.by.who is Person && !a.by.check_key
    && may_act(a.by, Role::Member)
    && !work_of(i, c, a.by.key)
}

/// Approval j is the first one by its key that reviews this change.
pub open spec fn first_review(i: Input, c: Cmd, j: int) -> bool {
    reviews(i, c, i.approvals[j])
    && !exists|k: int| 0 <= k < j && reviews(i, c, i.approvals[k])
                       && i.approvals[k].by.key == i.approvals[j].by.key
}

/// Distinct keys among the first n approvals that review this change.
pub open spec fn reviewers(i: Input, c: Cmd, n: int) -> nat
    decreases n,
{
    if n <= 0 { 0 } else { reviewers(i, c, n - 1) + if first_review(i, c, n - 1) { 1nat } else { 0nat } }
}

pub open spec fn trusted_for(i: Input, check: u64, key: u64) -> bool {
    exists|j: int| 0 <= j < i.trusted.len() && i.trusted[j].check == check && i.trusted[j].key == key
}

pub open spec fn trusted_result(i: Input, check: u64, head: u64, q: Post) -> bool {
    q.check == check && q.head == head && trusted_for(i, check, q.by)
}

/// A trusted reporter's newest result for exactly this commit is a pass.
pub open spec fn latest_trusted_pass(i: Input, check: u64, head: u64) -> bool {
    exists|j: int| 0 <= j < i.posts.len() && trusted_result(i, check, head, i.posts[j]) && i.posts[j].pass
        && forall|k: int| 0 <= k < i.posts.len() && k != j && trusted_result(i, check, head, i.posts[k])
               ==> i.posts[k].seq < i.posts[j].seq
}

pub open spec fn is_create(c: Cmd) -> bool { c.old == NO_COMMIT }
pub open spec fn is_delete(c: Cmd) -> bool { c.old != NO_COMMIT && c.new == NO_COMMIT }
pub open spec fn is_nonff(c: Cmd) -> bool { !is_create(c) && !is_delete(c) && !c.fast_forward }

/// Without rules: members create refs and fast-forward branches.
pub open spec fn baseline(c: Cmd) -> Role {
    if is_create(c) || (!is_delete(c) && c.fast_forward && c.is_branch) { Role::Member } else { Role::Admin }
}

pub open spec fn satisfied(i: Input, c: Cmd, x: Rule) -> bool {
    match x.req {
        Req::AdminPush => may_act(i.pusher, Role::Admin),
        Req::NoForce => !is_nonff(c),
        Req::NoDelete => !is_delete(c),
        Req::Review => reviewers(i, c, i.approvals.len() as int) >= x.n,
        Req::FreshTarget => true,
        Req::Check => latest_trusted_pass(i, x.check, c.new),
        Req::CodeOwners => c.owners.len() == 0
            || exists|j: int| 0 <= j < i.approvals.len() && reviews(i, c, i.approvals[j])
                   && (exists|k: int| 0 <= k < c.owners.len() && c.owners[k] == i.approvals[j].by.key),
        Req::Unknown => false,
    }
}

pub open spec fn ref_change_ok(i: Input, c: Cmd) -> bool {
    c.old == c.current
    && may_act(i.pusher, baseline(c))
    && (forall|j: int| 0 <= j < i.floor.len() && floor_governs(i, c, i.floor[j]) ==> satisfied(i, c, i.floor[j]))
    && (forall|j: int| 0 <= j < i.repo_rules.len() && i.repo_rules[j].git_ref == c.git_ref
           ==> satisfied(i, c, i.repo_rules[j]))
}

/// A push lands whole or not at all, and only clean.
pub open spec fn lands_ok(i: Input) -> bool {
    i.scan == Scan::Clean && i.cmds.len() > 0
    && forall|j: int| 0 <= j < i.cmds.len() ==> ref_change_ok(i, i.cmds[j])
}

// ---------------------------------------------------------------------------
// The evaluator.
// ---------------------------------------------------------------------------

/// The role the relay credits k with: its channel role, and for an agent,
/// capped by its owner's, only under a live grant.
fn credited(k: KeyFacts) -> (out: Role)
    ensures forall|n: Role| n != Role::None ==> (rank(out) >= rank(n) <==> may_act(k, n)),
{
    if !k.admitted { return Role::None; }
    match k.who {
        Who::Person => k.role,
        Who::UnownedAgent => Role::None,
        Who::Agent(o) => {
            if !k.granted || !o.admitted { Role::None }
            else if rank_of(o.role) < rank_of(k.role) { o.role } else { k.role }
        }
    }
}

/// Callers only ask for a real level; every key trivially reaches `None`.
fn holds(k: KeyFacts, n: Role) -> (out: bool)
    requires n != Role::None,
    ensures out == may_act(k, n),
{
    rank_of(credited(k)) >= rank_of(n)
}

fn principal(k: KeyFacts) -> (out: u64)
    ensures out == person(k),
{
    match k.who { Who::Agent(o) => o.key, _ => k.key }
}

fn contains(v: &[u64], x: u64) -> (out: bool)
    ensures out == exists|j: int| 0 <= j < v.len() && v[j] == x,
{
    let mut j: usize = 0;
    while j < v.len()
        invariant j <= v.len(), forall|k: int| 0 <= k < j ==> v[k] != x,
        decreases v.len() - j,
    {
        if v[j] == x { return true; }
        j += 1;
    }
    false
}

fn floor_on(i: &Input, c: &Cmd, x: Rule) -> (out: bool)
    ensures out == floor_governs(*i, *c, x),
{
    x.git_ref == c.git_ref && match x.scope { Scope::All => true, Scope::Label(l) => contains(&i.labels, l) }
}

fn is_fresh(x: Rule) -> (out: bool)
    ensures out == (x.req == Req::FreshTarget),
{
    matches!(x.req, Req::FreshTarget)
}

fn wants_fresh(i: &Input, c: &Cmd) -> (out: bool)
    ensures out == fresh_target(*i, *c),
{
    let mut j: usize = 0;
    while j < i.floor.len()
        invariant j <= i.floor.len(),
            forall|k: int| 0 <= k < j ==> !(floor_governs(*i, *c, i.floor[k]) && i.floor[k].req == Req::FreshTarget),
        decreases i.floor.len() - j,
    {
        if floor_on(i, c, i.floor[j]) && is_fresh(i.floor[j]) { return true; }
        j += 1;
    }
    let mut j: usize = 0;
    while j < i.repo_rules.len()
        invariant j <= i.repo_rules.len(),
            forall|k: int| 0 <= k < i.floor.len() ==> !(floor_governs(*i, *c, i.floor[k]) && i.floor[k].req == Req::FreshTarget),
            forall|k: int| 0 <= k < j ==> !(i.repo_rules[k].git_ref == c.git_ref && i.repo_rules[k].req == Req::FreshTarget),
        decreases i.repo_rules.len() - j,
    {
        if i.repo_rules[j].git_ref == c.git_ref && is_fresh(i.repo_rules[j]) { return true; }
        j += 1;
    }
    false
}

/// Whether approval a fills a human slot for this change.
fn counts(i: &Input, c: &Cmd, a: Approval, fresh: bool) -> (out: bool)
    requires fresh == fresh_target(*i, *c),
    ensures out == reviews(*i, *c, a),
{
    if a.repo != i.repo || a.git_ref != c.git_ref || a.head != c.new { return false; }
    if fresh && a.base != c.old { return false; }
    let is_person = matches!(a.by.who, Who::Person);
    if !is_person || a.by.check_key { return false; }
    if !holds(a.by, Role::Member) { return false; }
    let own_work = contains(&c.authors, a.by.key) || (c.brings && principal(i.pusher) == a.by.key);
    !own_work
}

fn counted_before(i: &Input, c: &Cmd, fresh: bool, j: usize) -> (out: bool)
    requires fresh == fresh_target(*i, *c), j < i.approvals.len(),
    ensures out == exists|k: int| 0 <= k < j && reviews(*i, *c, i.approvals[k])
                                  && i.approvals[k].by.key == i.approvals[j as int].by.key,
{
    let mut k: usize = 0;
    while k < j
        invariant k <= j, j < i.approvals.len(), fresh == fresh_target(*i, *c),
            forall|m: int| 0 <= m < k ==> !(reviews(*i, *c, i.approvals[m])
                                           && i.approvals[m].by.key == i.approvals[j as int].by.key),
        decreases j - k,
    {
        if i.approvals[k].by.key == i.approvals[j].by.key && counts(i, c, i.approvals[k], fresh) { return true; }
        k += 1;
    }
    false
}

/// Distinct keys whose approvals fill a human slot for this change.
fn count_reviewers(i: &Input, c: &Cmd, fresh: bool) -> (n: u64)
    requires fresh == fresh_target(*i, *c), i.approvals.len() < u64::MAX,
    ensures n as nat == reviewers(*i, *c, i.approvals.len() as int),
{
    let mut n: u64 = 0;
    let mut j: usize = 0;
    while j < i.approvals.len()
        invariant j <= i.approvals.len(), i.approvals.len() < u64::MAX, fresh == fresh_target(*i, *c),
            n as nat == reviewers(*i, *c, j as int), n <= j,
        decreases i.approvals.len() - j,
    {
        if counts(i, c, i.approvals[j], fresh) && !counted_before(i, c, fresh, j) { n += 1; }
        j += 1;
    }
    n
}

fn owner_reviewed(i: &Input, c: &Cmd, fresh: bool) -> (out: bool)
    requires fresh == fresh_target(*i, *c),
    ensures out == exists|j: int| 0 <= j < i.approvals.len() && reviews(*i, *c, i.approvals[j])
                       && (exists|k: int| 0 <= k < c.owners.len() && c.owners[k] == i.approvals[j].by.key),
{
    let mut j: usize = 0;
    while j < i.approvals.len()
        invariant j <= i.approvals.len(), fresh == fresh_target(*i, *c),
            forall|m: int| 0 <= m < j ==> !(reviews(*i, *c, i.approvals[m])
                && (exists|k: int| 0 <= k < c.owners.len() && c.owners[k] == i.approvals[m].by.key)),
        decreases i.approvals.len() - j,
    {
        if counts(i, c, i.approvals[j], fresh) && contains(&c.owners, i.approvals[j].by.key) { return true; }
        j += 1;
    }
    false
}

fn is_trusted(i: &Input, check: u64, key: u64) -> (out: bool)
    ensures out == trusted_for(*i, check, key),
{
    let mut j: usize = 0;
    while j < i.trusted.len()
        invariant j <= i.trusted.len(),
            forall|k: int| 0 <= k < j ==> !(i.trusted[k].check == check && i.trusted[k].key == key),
        decreases i.trusted.len() - j,
    {
        if i.trusted[j].check == check && i.trusted[j].key == key { return true; }
        j += 1;
    }
    false
}

fn trusted_post(i: &Input, check: u64, head: u64, q: Post) -> (out: bool)
    ensures out == trusted_result(*i, check, head, q),
{
    q.check == check && q.head == head && is_trusted(i, check, q.by)
}

/// No other trusted result for this check and commit is as new as post j.
fn newest(i: &Input, check: u64, head: u64, j: usize) -> (out: bool)
    requires j < i.posts.len(),
    ensures out == forall|k: int| 0 <= k < i.posts.len() && k != j && trusted_result(*i, check, head, i.posts[k])
                       ==> i.posts[k].seq < i.posts[j as int].seq,
{
    let mut k: usize = 0;
    while k < i.posts.len()
        invariant k <= i.posts.len(), j < i.posts.len(),
            forall|m: int| 0 <= m < k && m != j && trusted_result(*i, check, head, i.posts[m])
                ==> i.posts[m].seq < i.posts[j as int].seq,
        decreases i.posts.len() - k,
    {
        if k != j && trusted_post(i, check, head, i.posts[k]) && i.posts[k].seq >= i.posts[j].seq { return false; }
        k += 1;
    }
    true
}

fn check_passes(i: &Input, check: u64, head: u64) -> (out: bool)
    ensures out == latest_trusted_pass(*i, check, head),
{
    let mut j: usize = 0;
    while j < i.posts.len()
        invariant j <= i.posts.len(),
            forall|m: int| 0 <= m < j ==> !(trusted_result(*i, check, head, i.posts[m]) && i.posts[m].pass
                && forall|k: int| 0 <= k < i.posts.len() && k != m && trusted_result(*i, check, head, i.posts[k])
                       ==> i.posts[k].seq < i.posts[m].seq),
        decreases i.posts.len() - j,
    {
        if trusted_post(i, check, head, i.posts[j]) && i.posts[j].pass && newest(i, check, head, j) { return true; }
        j += 1;
    }
    false
}

fn meets(i: &Input, c: &Cmd, x: Rule, fresh: bool) -> (out: bool)
    requires fresh == fresh_target(*i, *c), i.approvals.len() < u64::MAX,
    ensures out == satisfied(*i, *c, x),
{
    let create = c.old == NO_COMMIT;
    let delete = !create && c.new == NO_COMMIT;
    match x.req {
        Req::AdminPush => holds(i.pusher, Role::Admin),
        Req::NoForce => create || delete || c.fast_forward,
        Req::NoDelete => !delete,
        Req::Review => count_reviewers(i, c, fresh) >= x.n,
        Req::FreshTarget => true,
        Req::Check => check_passes(i, x.check, c.new),
        Req::CodeOwners => c.owners.is_empty() || owner_reviewed(i, c, fresh),
        Req::Unknown => false,
    }
}

/// Every rule in force for the ref: the floor's for every repo and for the
/// repo's protected labels, and the repo's own. None replaces another.
fn rules_hold(i: &Input, c: &Cmd) -> (out: bool)
    requires i.approvals.len() < u64::MAX,
    ensures out == ((forall|j: int| 0 <= j < i.floor.len() && floor_governs(*i, *c, i.floor[j])
                         ==> satisfied(*i, *c, i.floor[j]))
                    && (forall|j: int| 0 <= j < i.repo_rules.len() && i.repo_rules[j].git_ref == c.git_ref
                         ==> satisfied(*i, *c, i.repo_rules[j]))),
{
    let fresh = wants_fresh(i, c);
    let mut j: usize = 0;
    while j < i.floor.len()
        invariant j <= i.floor.len(), fresh == fresh_target(*i, *c), i.approvals.len() < u64::MAX,
            forall|k: int| 0 <= k < j && floor_governs(*i, *c, i.floor[k]) ==> satisfied(*i, *c, i.floor[k]),
        decreases i.floor.len() - j,
    {
        if floor_on(i, c, i.floor[j]) && !meets(i, c, i.floor[j], fresh) { return false; }
        j += 1;
    }
    let mut j: usize = 0;
    while j < i.repo_rules.len()
        invariant j <= i.repo_rules.len(), fresh == fresh_target(*i, *c), i.approvals.len() < u64::MAX,
            forall|k: int| 0 <= k < i.floor.len() && floor_governs(*i, *c, i.floor[k]) ==> satisfied(*i, *c, i.floor[k]),
            forall|k: int| 0 <= k < j && i.repo_rules[k].git_ref == c.git_ref ==> satisfied(*i, *c, i.repo_rules[k]),
        decreases i.repo_rules.len() - j,
    {
        if i.repo_rules[j].git_ref == c.git_ref && !meets(i, c, i.repo_rules[j], fresh) { return false; }
        j += 1;
    }
    true
}

/// One ref change: the ref still names what the push replaces, the pusher
/// may make this kind of change, and every rule in force for the ref holds.
pub fn ref_ok(i: &Input, c: &Cmd) -> (out: bool)
    requires i.approvals.len() < u64::MAX,
    ensures out == ref_change_ok(*i, *c),
{
    let need = if c.old == NO_COMMIT || (c.new != NO_COMMIT && c.fast_forward && c.is_branch) {
        Role::Member
    } else {
        Role::Admin
    };
    c.old == c.current && holds(i.pusher, need) && rules_hold(i, c)
}

/// The decision for a whole push: every ref change, or none.
pub fn decide(i: &Input) -> (allow: bool)
    requires i.approvals.len() < u64::MAX,
    ensures allow == lands_ok(*i),
{
    match i.scan { Scan::Clean => {}, _ => return false }
    if i.cmds.is_empty() { return false; }
    let mut j: usize = 0;
    while j < i.cmds.len()
        invariant j <= i.cmds.len(), i.approvals.len() < u64::MAX,
            forall|k: int| 0 <= k < j ==> ref_change_ok(*i, i.cmds[k]),
        decreases i.cmds.len() - j,
    {
        if !ref_ok(i, &i.cmds[j]) { return false; }
        j += 1;
    }
    true
}

} // verus!

#[cfg(test)]
mod tests;
