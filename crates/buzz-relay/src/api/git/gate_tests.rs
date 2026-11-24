//! The mapping from governance documents and evidence to the gate's input.
//! Every case names the seed it rules out.

use super::*;
use buzz_core::governance::Evidence;
use buzz_db::git_land::Facts;

const PUSHER: &str = "11111111111111111111111111111111111111111111111111111111111111aa";
const OWNER: &str = "22222222222222222222222222222222222222222222222222222222222222bb";
const ALICE: &str = "33333333333333333333333333333333333333333333333333333333333333cc";
const BOB: &str = "44444444444444444444444444444444444444444444444444444444444444dd";
const CI: &str = "55555555555555555555555555555555555555555555555555555555555555ee";
const OLD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const NEW: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const MAIN: &str = "refs/heads/main";

fn repo() -> RepoCoordinate {
    format!("30617:{}:widget", "ab".repeat(32)).parse().unwrap()
}

fn facts(role: &str) -> Facts {
    Facts {
        banned: false,
        community_member: true,
        channel_role: Some(role.into()),
    }
}

fn person(role: &str) -> git_land::KeyFacts {
    git_land::KeyFacts {
        key: facts(role),
        owner: None,
    }
}

fn agent(role: &str, owner_role: &str) -> git_land::KeyFacts {
    git_land::KeyFacts {
        key: facts(role),
        owner: Some((hex::decode(OWNER).unwrap(), facts(owner_role))),
    }
}

fn rule(refs: &[&str], require: Vec<Requirement>) -> DocRule {
    DocRule {
        id: "r".into(),
        refs: refs.iter().map(|r| r.to_string()).collect(),
        labels: BTreeMap::new(),
        require,
    }
}

fn sourced<T>(value: T) -> Sourced<T> {
    Sourced { value, head: None }
}

fn policy(floor: Vec<DocRule>, rules: Vec<DocRule>) -> Policy {
    Policy {
        floor: sourced((floor, BTreeMap::new())),
        trusted: sourced(BTreeMap::from([(
            "ci".to_string(),
            BTreeSet::from([CI.to_string()]),
        )])),
        labels: sourced(BTreeMap::new()),
        rules: sourced(rules),
    }
}

fn ff() -> RefUpdate {
    RefUpdate {
        ref_name: MAIN.into(),
        kind: UpdateKind::FastForward,
        old_oid: OLD.into(),
        new_oid: NEW.into(),
    }
}

fn row(seq: i64, actor: &str, git_ref: &str, head: &str, body: EvidenceBody) -> EvidenceRow {
    EvidenceRow {
        seq,
        actor: actor.into(),
        evidence: Evidence {
            community: uuid::Uuid::nil(),
            repo: repo().to_string(),
            git_ref: git_ref.into(),
            head: head.into(),
            base: OLD.into(),
            body,
        },
    }
}

fn approve(seq: i64, by: &str) -> EvidenceRow {
    row(
        seq,
        by,
        MAIN,
        NEW,
        EvidenceBody::Approval { approved: true },
    )
}

fn check(seq: i64, by: &str, name: &str, passed: bool) -> EvidenceRow {
    row(
        seq,
        by,
        MAIN,
        NEW,
        EvidenceBody::Check {
            name: name.into(),
            passed,
        },
    )
}

fn review(n: u32) -> Requirement {
    Requirement::Review {
        count: n,
        group: None,
    }
}

struct Case {
    policy: Policy,
    pusher: git_land::KeyFacts,
    grant: Option<buzz_core::channel::MemberRole>,
    updates: Vec<RefUpdate>,
    scan: gate::Scan,
    rows: Vec<EvidenceRow>,
    approvers: HashMap<String, git_land::KeyFacts>,
}

impl Case {
    fn new(policy: Policy) -> Self {
        Self {
            policy,
            pusher: person("member"),
            grant: None,
            updates: vec![ff()],
            scan: gate::Scan::Clean,
            rows: Vec::new(),
            approvers: [ALICE, BOB, CI, PUSHER]
                .into_iter()
                .map(|k| (k.to_string(), person("member")))
                .collect(),
        }
    }
    fn decide(&self) -> Decision {
        let pusher = Principal {
            key: PUSHER.into(),
            facts: &self.pusher,
            grant: self.grant,
        };
        assemble(
            &self.policy,
            &repo(),
            &pusher,
            &self.updates,
            self.scan,
            &self.rows,
            &self.approvers,
            false,
        )
        .unwrap()
    }
    fn lands(&self) -> bool {
        self.decide().verdict.is_ok()
    }
}

#[test]
fn no_governance_leaves_the_baseline() {
    assert!(Case::new(policy(vec![], vec![])).lands());
    let mut c = Case::new(policy(vec![], vec![]));
    c.updates[0].kind = UpdateKind::NonFastForward;
    assert!(!c.lands(), "baseline: a member may not force-push");
}

/// Direct membership and capped delegation are independent, including on
/// branches that require admin authority.
#[test]
fn agents_use_membership_or_capped_grants() {
    use buzz_core::channel::MemberRole;
    let mut c = Case::new(policy(
        vec![rule(&[MAIN], vec![Requirement::AdminPush])],
        vec![],
    ));
    c.pusher = agent("admin", "member");
    assert!(c.lands(), "direct admin needs no grant or owner admin role");
    c.grant = Some(MemberRole::Guest);
    assert!(c.lands(), "read grant cannot reduce direct admin");
    c.pusher = agent("guest", "admin");
    assert!(!c.lands(), "read grant cannot delegate admin");
    c.grant = Some(MemberRole::Member);
    assert!(!c.lands(), "push grant cannot delegate admin");
    c.grant = Some(MemberRole::Admin);
    assert!(c.lands(), "manage grant raises guest to admin");
    c.pusher.key.channel_role = None;
    assert!(c.lands(), "grant-only admin");
    c.pusher.owner.as_mut().unwrap().1.channel_role = Some("member".into());
    assert!(!c.lands(), "delegation cannot exceed owner");
    c.pusher.owner.as_mut().unwrap().1.channel_role = Some("admin".into());
    c.grant = None;
    assert!(!c.lands(), "neither route");
}

/// Seeds stale-approval, self-approval, duplicate-reviewer-counts,
/// withdrawal folding.
#[test]
fn review_counts_distinct_live_approvals_of_the_exact_tip() {
    let p = || policy(vec![], vec![rule(&[MAIN], vec![review(1)])]);
    let mut c = Case::new(p());
    assert!(!c.lands(), "no approval");
    c.rows = vec![approve(1, ALICE)];
    assert!(c.lands());
    let d = c.decide();
    assert_eq!(d.counted[MAIN].evidence, BTreeSet::from(["1".to_string()]));

    c.rows = vec![row(
        1,
        ALICE,
        MAIN,
        OLD,
        EvidenceBody::Approval { approved: true },
    )];
    assert!(!c.lands(), "stale-approval: another tip");
    c.rows = vec![row(
        1,
        ALICE,
        "refs/heads/dev",
        NEW,
        EvidenceBody::Approval { approved: true },
    )];
    assert!(!c.lands(), "evidence-any-ref: another ref");
    c.rows = vec![approve(1, PUSHER)];
    assert!(!c.lands(), "self-approval");

    c.rows = vec![
        approve(1, ALICE),
        row(
            2,
            ALICE,
            MAIN,
            NEW,
            EvidenceBody::Approval { approved: false },
        ),
    ];
    assert!(!c.lands(), "a later withdrawal leaves no approval");
    c.rows = vec![
        row(
            1,
            ALICE,
            MAIN,
            NEW,
            EvidenceBody::Approval { approved: false },
        ),
        approve(2, ALICE),
    ];
    assert!(c.lands(), "a later approval stands");

    let mut c = Case::new(policy(vec![], vec![rule(&[MAIN], vec![review(2)])]));
    c.rows = vec![approve(1, ALICE), approve(2, ALICE)];
    assert!(!c.lands(), "duplicate-reviewer-counts");
    c.rows = vec![approve(1, ALICE), approve(2, BOB)];
    assert!(c.lands());
}

/// Seeds agent-approves, ci-as-human, eligibility-at-approve.
#[test]
fn approvers_are_judged_by_their_standing_now() {
    let mut c = Case::new(policy(vec![], vec![rule(&[MAIN], vec![review(1)])]));
    c.rows = vec![approve(1, ALICE)];
    c.approvers.insert(ALICE.into(), agent("admin", "admin"));
    assert!(!c.lands(), "agent-approves");
    c.rows = vec![approve(1, CI)];
    assert!(!c.lands(), "ci-as-human: a trusted check key");
    c.rows = vec![approve(1, BOB)];
    c.approvers.insert(BOB.into(), person("guest"));
    assert!(!c.lands(), "eligibility-at-approve: demoted to guest");
    let mut banned = person("member");
    banned.key.banned = true;
    c.approvers.insert(BOB.into(), banned);
    assert!(!c.lands(), "eligibility-at-approve: banned");
}

/// Seeds untrusted-check, trusted-for-any-check, first-pass-wins.
#[test]
fn checks_take_the_latest_trusted_result() {
    let req = || vec![Requirement::Check { name: "ci".into() }];
    let mut c = Case::new(policy(vec![], vec![rule(&[MAIN], req())]));
    assert!(!c.lands());
    c.rows = vec![check(1, ALICE, "ci", true)];
    assert!(!c.lands(), "untrusted-check");
    c.policy
        .trusted
        .value
        .insert("lint".into(), BTreeSet::from([ALICE.into()]));
    assert!(!c.lands(), "trusted-for-any-check");
    c.rows = vec![check(1, CI, "ci", true)];
    assert!(c.lands());
    assert_eq!(
        c.decide().counted[MAIN].checks,
        BTreeSet::from(["1".to_string()])
    );
    c.rows = vec![check(1, CI, "ci", true), check(2, CI, "ci", false)];
    assert!(!c.lands(), "first-pass-wins");
    c.rows = vec![check(1, CI, "ci", false), check(2, CI, "ci", true)];
    assert!(c.lands());
}

/// Seed skip-unknown: what the gate has no input for denies.
#[test]
fn requirements_without_input_deny() {
    for req in [
        Requirement::Codeowners,
        Requirement::Review {
            count: 1,
            group: Some(uuid::Uuid::nil()),
        },
    ] {
        let mut c = Case::new(policy(vec![], vec![rule(&[MAIN], vec![req.clone()])]));
        c.rows = vec![approve(1, ALICE), approve(2, BOB)];
        assert!(!c.lands(), "{req:?}");
    }
    let c = Case::new(policy(vec![], vec![rule(&["not-a-ref", MAIN], vec![])]));
    assert!(!c.lands(), "an unparseable pattern governs every ref");
}

/// Approvals must have seen the target tip unless the rule relaxes it.
#[test]
fn fresh_target_is_the_default() {
    let mut c = Case::new(policy(vec![], vec![rule(&[MAIN], vec![review(1)])]));
    let mut a = approve(1, ALICE);
    a.evidence.base = "cccccccccccccccccccccccccccccccccccccccc".into();
    c.rows = vec![a];
    assert!(!c.lands(), "target-tip-ignored");
    c.policy.rules.value[0]
        .require
        .push(Requirement::FreshTarget { required: false });
    assert!(c.lands());
}

/// Seed repo-overrides-floor: a repo cannot loosen the floor.
#[test]
fn the_floor_binds_whatever_the_repo_says() {
    let mut c = Case::new(policy(
        vec![rule(&[MAIN], vec![Requirement::NoForce])],
        vec![],
    ));
    c.updates[0].kind = UpdateKind::NonFastForward;
    c.pusher = person("admin");
    assert!(!c.lands());
    c.policy.floor.value.0.clear();
    assert!(c.lands(), "control: an admin may force without the floor");
}

/// A floor rule scoped by a label binds by the floor's default when the
/// repo has no labels head; without that it would fail open.
#[test]
fn label_scoped_floor_rules_follow_the_labels() {
    let mut scoped = rule(&[MAIN], vec![review(1)]);
    scoped.labels = BTreeMap::from([("tier".to_string(), "prod".to_string())]);
    let mut c = Case::new(policy(vec![scoped], vec![]));
    assert!(c.lands(), "unlabelled: the rule does not apply");
    c.policy.labels.value.insert("tier".into(), "prod".into());
    assert!(!c.lands());
    c.policy.labels.value.insert("tier".into(), "dev".into());
    assert!(c.lands());
}

/// Seed any-ref-decides: one denied ref refuses the push and is named.
#[test]
fn one_denied_ref_refuses_the_push() {
    let mut c = Case::new(policy(
        vec![],
        vec![rule(&["refs/heads/release"], vec![review(1)])],
    ));
    c.updates.push(RefUpdate {
        ref_name: "refs/heads/release".into(),
        ..ff()
    });
    let denied = c.decide().verdict.unwrap_err();
    assert_eq!(
        denied
            .iter()
            .map(|d| d.ref_name.as_str())
            .collect::<Vec<_>>(),
        ["refs/heads/release"]
    );
}

/// Seeds scan-governed-only and visible-before-scan: a scan that is not
/// clean refuses every ref of the push, governed or not, and says why.
#[test]
fn a_scan_that_is_not_clean_refuses_every_ref() {
    for (scan, reason) in [
        (gate::Scan::Dirty, SECRET_DETECTED),
        (gate::Scan::Unknown, SCAN_INCOMPLETE),
    ] {
        let mut c = Case::new(policy(vec![], vec![]));
        c.updates.push(RefUpdate {
            ref_name: "refs/heads/scratch".into(),
            ..ff()
        });
        assert!(c.lands(), "ungoverned and clean");
        c.scan = scan;
        let denied = c.decide().verdict.unwrap_err();
        assert_eq!(
            denied
                .iter()
                .map(|d| (d.ref_name.as_str(), d.reason.as_str()))
                .collect::<Vec<_>>(),
            [(MAIN, reason), ("refs/heads/scratch", reason)]
        );
    }
}

#[test]
fn stored_roles_rank_fail_closed() {
    assert_eq!(role(Some("owner")), gate::Role::Admin);
    assert_eq!(role(Some("bot")), gate::Role::Member);
    assert_eq!(role(Some("moderator")), gate::Role::None);
    assert_eq!(role(None), gate::Role::None);
}

#[test]
fn a_restricted_relay_admits_an_agent_through_its_owner() {
    let mut ids = Interner::default();
    let outsider = Facts {
        community_member: false,
        ..facts("admin")
    };
    let mut admitted = |read: &git_land::KeyFacts| {
        key_facts(
            &mut ids,
            PUSHER,
            read,
            Some(buzz_core::channel::MemberRole::Member),
            false,
            &BTreeSet::new(),
        )
        .admitted
    };
    let via_owner = git_land::KeyFacts {
        key: outsider.clone(),
        ..agent("admin", "admin")
    };
    assert!(admitted(&via_owner), "its owner's membership admits it");
    let banned = git_land::KeyFacts {
        key: Facts {
            banned: true,
            ..outsider.clone()
        },
        ..via_owner.clone()
    };
    assert!(!admitted(&banned), "a ban is never vouched away");
    let stray = git_land::KeyFacts {
        owner: Some((hex::decode(OWNER).unwrap(), outsider.clone())),
        ..via_owner
    };
    assert!(!admitted(&stray), "an outside owner admits nothing");
    let person = git_land::KeyFacts {
        key: outsider,
        owner: None,
    };
    assert!(!admitted(&person), "a person needs its own membership");
}
