//! Concrete cases, written from the model's scenarios rather than from
//! `lands_ok`, so a defect shared by the evaluator and its spec still fails.
use super::*;

const REPO: u64 = 1;
const MAIN: u64 = 10;
const C0: u64 = 100;
const C1: u64 = 101;
const C2: u64 = 102;
const CI: u64 = 7;

fn person(key: u64, role: Role) -> KeyFacts {
    KeyFacts {
        key,
        admitted: true,
        role,
        who: Who::Person,
        granted: false,
        check_key: false,
    }
}

fn agent(key: u64, role: Role, owner: u64, owner_role: Role) -> KeyFacts {
    KeyFacts {
        key,
        admitted: true,
        role,
        who: Who::Agent(Owner {
            key: owner,
            admitted: true,
            role: owner_role,
        }),
        granted: true,
        check_key: false,
    }
}

fn rule(req: Req) -> Rule {
    Rule {
        req,
        git_ref: MAIN,
        scope: Scope::All,
        n: 1,
        check: CI,
    }
}

fn ff() -> Cmd {
    Cmd {
        git_ref: MAIN,
        is_branch: true,
        old: C0,
        current: C0,
        new: C1,
        fast_forward: true,
        brings: true,
        authors: vec![],
        owners: vec![],
    }
}

fn approval(by: KeyFacts) -> Approval {
    Approval {
        by,
        repo: REPO,
        git_ref: MAIN,
        head: C1,
        base: C0,
    }
}

fn push(pusher: KeyFacts, floor: Vec<Rule>) -> Input {
    Input {
        repo: REPO,
        pusher,
        scan: Scan::Clean,
        cmds: vec![ff()],
        floor,
        labels: vec![],
        repo_rules: vec![],
        trusted: vec![Reporter { check: CI, key: 50 }],
        approvals: vec![],
        posts: vec![],
    }
}

fn reviewed(pusher: KeyFacts, reviewer: KeyFacts) -> Input {
    let mut i = push(pusher, vec![rule(Req::Review)]);
    i.approvals.push(approval(reviewer));
    i
}

#[test]
fn member_fast_forwards_without_rules() {
    assert!(decide(&push(person(2, Role::Member), vec![])));
    assert!(!decide(&push(person(2, Role::Guest), vec![])));
}

#[test]
fn a_moved_ref_or_a_dirty_scan_denies() {
    let mut i = push(person(2, Role::Member), vec![]);
    i.cmds[0].current = C2;
    assert!(!decide(&i));
    for scan in [Scan::Dirty, Scan::Unknown] {
        let mut i = push(person(2, Role::Member), vec![]);
        i.scan = scan;
        assert!(!decide(&i));
    }
}

#[test]
fn agent_is_capped_by_owner_and_needs_a_grant() {
    // The lesser of the two roles, from either side.
    assert!(!decide(&push(
        agent(3, Role::Admin, 2, Role::Guest),
        vec![]
    )));
    assert!(!decide(&push(
        agent(3, Role::Guest, 2, Role::Admin),
        vec![]
    )));
    let mut g = agent(3, Role::Member, 2, Role::Member);
    assert!(decide(&push(g, vec![])));
    g.granted = false;
    assert!(!decide(&push(g, vec![])));
    let unowned = KeyFacts {
        who: Who::UnownedAgent,
        ..person(3, Role::Admin)
    };
    assert!(!decide(&push(unowned, vec![])));
}

#[test]
fn review_counts_only_an_eligible_independent_person() {
    let b = person(2, Role::Member);
    assert!(decide(&reviewed(b, person(4, Role::Member))));
    // self, the agent's owner, an agent, a check key, a guest
    assert!(!decide(&reviewed(b, b)));
    assert!(!decide(&reviewed(
        agent(3, Role::Member, 2, Role::Member),
        b
    )));
    assert!(!decide(&reviewed(
        b,
        agent(5, Role::Member, 4, Role::Member)
    )));
    assert!(!decide(&reviewed(
        b,
        KeyFacts {
            check_key: true,
            ..person(4, Role::Member)
        }
    )));
    assert!(!decide(&reviewed(b, person(4, Role::Guest))));
}

#[test]
fn review_is_bound_to_repo_ref_and_head() {
    let b = person(2, Role::Member);
    let d = person(4, Role::Member);
    for bad in [
        Approval {
            repo: 9,
            ..approval(d)
        },
        Approval {
            git_ref: 11,
            ..approval(d)
        },
        Approval {
            head: C0,
            ..approval(d)
        },
    ] {
        let mut i = push(b, vec![rule(Req::Review)]);
        i.approvals.push(bad);
        assert!(!decide(&i));
    }
}

#[test]
fn fresh_target_decides_whether_an_old_base_counts() {
    let b = person(2, Role::Member);
    let stale = Approval {
        base: C2,
        ..approval(person(4, Role::Member))
    };
    let mut i = push(b, vec![rule(Req::Review)]);
    i.approvals.push(stale);
    assert!(decide(&i), "without fresh-target an old base still counts");
    i.floor.push(rule(Req::FreshTarget));
    assert!(!decide(&i));
}

#[test]
fn two_approvals_by_one_key_are_one_reviewer() {
    let b = person(2, Role::Member);
    let d = person(4, Role::Member);
    let mut i = push(
        b,
        vec![Rule {
            n: 2,
            ..rule(Req::Review)
        }],
    );
    i.approvals = vec![approval(d), approval(d)];
    assert!(!decide(&i));
    i.approvals.push(approval(person(5, Role::Member)));
    assert!(decide(&i));
}

#[test]
fn the_newest_trusted_result_decides_a_check() {
    let mut i = push(person(2, Role::Member), vec![rule(Req::Check)]);
    let post = |by, pass, seq| Post {
        by,
        check: CI,
        head: C1,
        pass,
        seq,
    };
    assert!(!decide(&i));
    i.posts = vec![post(60, true, 0)];
    assert!(!decide(&i), "an untrusted pass does not count");
    i.posts = vec![post(50, true, 0), post(50, false, 1)];
    assert!(!decide(&i), "a later failure supersedes a pass");
    i.posts = vec![post(50, false, 0), post(50, true, 1)];
    assert!(decide(&i));
    i.trusted = vec![Reporter { check: 8, key: 50 }];
    assert!(!decide(&i), "trusted for another check");
}

#[test]
fn code_owners_come_from_the_input_old_tip() {
    let b = person(2, Role::Member);
    let mut i = reviewed(b, person(4, Role::Member));
    i.floor = vec![rule(Req::CodeOwners)];
    i.cmds[0].owners = vec![6];
    assert!(!decide(&i));
    i.cmds[0].owners = vec![4];
    assert!(decide(&i));
}

#[test]
fn unknown_rules_deny_and_repo_rules_only_add() {
    let b = person(2, Role::Member);
    assert!(!decide(&push(b, vec![rule(Req::Unknown)])));
    let mut i = push(b, vec![rule(Req::Review)]);
    i.repo_rules = vec![rule(Req::NoForce)];
    assert!(
        !decide(&i),
        "a repo rule does not replace the floor's review"
    );
}

#[test]
fn floor_applies_to_tags_and_to_labelled_repos() {
    let a = person(1, Role::Admin);
    let mut i = push(
        a,
        vec![Rule {
            scope: Scope::Label(3),
            ..rule(Req::NoDelete)
        }],
    );
    i.cmds[0] = Cmd {
        is_branch: false,
        new: NO_COMMIT,
        ..ff()
    };
    assert!(decide(&i), "the label is not on this repo");
    i.labels = vec![3];
    assert!(!decide(&i), "a tag is governed like a branch");
}

#[test]
fn a_push_lands_whole_or_not_at_all() {
    let mut i = push(person(2, Role::Member), vec![]);
    i.cmds.push(Cmd {
        git_ref: 11,
        fast_forward: false,
        ..ff()
    });
    assert!(!decide(&i), "the non-fast-forward ref needs an admin");
    i.cmds.clear();
    assert!(!decide(&i), "an empty push is not a land");
}

#[test]
fn moving_a_ref_to_an_already_shown_commit_makes_the_pusher_an_author() {
    // b moved feat to a shown commit; that push is in `authors` for main.
    let b = person(2, Role::Member);
    let mut i = reviewed(person(4, Role::Member), b);
    i.cmds[0].authors = vec![2];
    assert!(
        !decide(&i),
        "b brought the commit, so b's approval is not independent"
    );
    assert!(!counted_approval(&i, &i.cmds[0], i.approvals[0]));
    i.cmds[0].authors.clear();
    assert!(counted_approval(&i, &i.cmds[0], i.approvals[0]));
    // and the pusher of a change that brings anything is its author
    let mut i = reviewed(b, b);
    i.cmds[0].authors = vec![];
    assert!(!decide(&i));
}
