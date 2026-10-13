//! Work-trail acceptance cases (engineering plan v4 §7), on the production
//! read path: events come from the SDK builders, are signed, and are served
//! by a simulated origin that behaves like the Buzz relay — it applies only
//! its pushable constraints before `LIMIT` and every other tag constraint
//! after it, newest first.

use std::collections::BTreeSet;

use nostr::filter::MatchEventOptions;
use nostr::{Alphabet, Event, EventBuilder, EventId, Filter, Keys, Kind, Tag, Timestamp};
use serde_json::Value;
use uuid::Uuid;

use super::{
    build_work_label, build_work_part, build_work_request, AuthorityStatus as A, Contribution,
    ContributionRole as R, Entry, Independence, OutcomeScope, OwnerLink, PartSpec, RefStatus,
    Request, Response, Scope, Task, Trail, WorkRole, WorkTrail, MAX_PAGES, QUERY_LIMIT,
};
use crate::builders::{
    build_git_pr_update, build_git_pull_request, build_git_status, GitPrUpdateMeta,
    GitPullRequestMeta, GitRepoCoord, GitStatus, GitStatusMeta,
};
use crate::nip_oa::compute_auth_tag;

const ORIGIN: &str = "wss://relay.example";
const T0: u64 = 1_800_000_000;
const CUTOFF: u64 = T0 + 1_000_000;

/// A simulated Buzz relay.
#[derive(Default)]
struct Relay {
    events: Vec<Value>,
    keys: Option<Keys>,
    /// Refuse any filter asking for one of these kinds.
    refuse: Vec<u16>,
}

impl Relay {
    fn add(&mut self, event: &Event) {
        self.events
            .push(serde_json::to_value(event).expect("event json"));
    }

    fn tampered(&mut self, event: &Event) {
        let mut value = serde_json::to_value(event).expect("event json");
        value["content"] = Value::String(format!("{} (edited)", event.content));
        self.events.push(value);
    }

    /// What the Buzz relay does with one filter: pushable constraints and
    /// `LIMIT` in SQL, newest first; everything else afterwards.
    fn answer(&self, filter: &Filter) -> Vec<Value> {
        let mut pushed = filter.clone();
        let mut post = Filter::new();
        let nip33 = filter.kinds.as_ref().is_some_and(|k| {
            k.iter()
                .all(|k| buzz_core::kind::is_parameterized_replaceable(u32::from(k.as_u16())))
        });
        for (tag, values) in std::mem::take(&mut pushed.generic_tags) {
            let sql = !tag.uppercase
                && match tag.character {
                    Alphabet::H | Alphabet::E => true,
                    Alphabet::P => values.len() == 1,
                    Alphabet::D => nip33,
                    _ => false,
                };
            if sql {
                pushed.generic_tags.insert(tag, values);
            } else {
                post.generic_tags.insert(tag, values);
            }
        }
        let limit = pushed.limit.take().unwrap_or(QUERY_LIMIT);
        let mut rows: Vec<(Event, &Value)> = self
            .events
            .iter()
            .filter_map(|v| {
                serde_json::from_value::<Event>(v.clone())
                    .ok()
                    .map(|e| (e, v))
            })
            .filter(|(e, _)| pushed.match_event(e, MatchEventOptions::new()))
            .collect();
        rows.sort_by(|(a, _), (b, _)| b.created_at.cmp(&a.created_at).then(a.id.cmp(&b.id)));
        rows.into_iter()
            .take(limit)
            .filter(|(e, _)| post.match_event(e, MatchEventOptions::new()))
            .map(|(_, v)| v.clone())
            .collect()
    }
}

struct Run {
    trail: WorkTrail,
    filters: Vec<Filter>,
}

fn read(relay: &Relay, entry: Entry, scope: Scope) -> Run {
    read_at(relay, entry, scope, None)
}

/// [`read`], recording every answer as observed at `at` when given.
fn read_at(relay: &Relay, entry: Entry, scope: Scope, at: Option<u64>) -> Run {
    let mut trail = Trail::new(entry, scope);
    let mut filters = Vec::new();
    loop {
        let queries = trail.next_queries();
        if queries.is_empty() {
            break;
        }
        for q in queries {
            assert_eq!(q.origin, ORIGIN);
            let response = match &q.request {
                Request::RelayInfo => {
                    Response::RelayInfo(relay.keys.as_ref().map(Keys::public_key))
                }
                Request::Events(filter) => {
                    filters.push(filter.clone());
                    let refused = filter
                        .kinds
                        .as_ref()
                        .is_some_and(|k| k.iter().any(|k| relay.refuse.contains(&k.as_u16())));
                    if refused {
                        Response::Failed("restricted".into())
                    } else {
                        Response::Events(relay.answer(filter))
                    }
                }
            };
            match at {
                Some(at) => trail.ingest_at(q.scope, response, at),
                None => trail.ingest(q.scope, response),
            }
            .expect("ingest");
        }
    }
    Run {
        trail: trail.finish(),
        filters,
    }
}

fn scope(channel: Uuid, repos: &[String]) -> Scope {
    Scope {
        origin: ORIGIN.into(),
        channels: vec![channel.to_string()],
        repos: repos.to_vec(),
        since: None,
        until: CUTOFF,
    }
}

fn sign(builder: EventBuilder, keys: &Keys, at: u64) -> Event {
    builder
        .custom_created_at(Timestamp::from(at))
        .sign_with_keys(keys)
        .expect("sign")
}

fn auth(owner: &Keys, signer: &Keys) -> Tag {
    let json = compute_auth_tag(owner, &signer.public_key(), "").expect("auth tag");
    let parts: Vec<String> = serde_json::from_str(&json).expect("auth json");
    Tag::parse(parts).expect("auth tag")
}

fn criteria() -> Vec<String> {
    vec!["covers the three regions".into()]
}

fn request(channel: Uuid, keys: &Keys, at: u64) -> Event {
    let b = build_work_request(channel, "Draft the Q3 summary", "writing", &criteria())
        .expect("request");
    sign(b, keys, at)
}

fn part(request: &Event, role: WorkRole, keys: &Keys, at: u64, prior: Option<&Event>) -> Event {
    let spec = PartSpec {
        summary: format!("{role:?} by contributor"),
        state: (role == WorkRole::Update).then(|| "blocked".to_owned()),
        artifact: (role == WorkRole::Submission).then(|| "https://docs.example/q3".to_owned()),
        artifact_revision: (role == WorkRole::Submission).then(|| format!("rev-{at}")),
    };
    sign(
        build_work_part(request, role, &spec, prior).expect("part"),
        keys,
        at,
    )
}

fn label(target: &Event, claim: &str, keys: &Keys, at: u64) -> Event {
    let b = build_work_label(
        target,
        ORIGIN,
        claim,
        "checked against criteria",
        &criteria(),
        &[],
    )
    .expect("label");
    sign(b, keys, at)
}

fn only_task(trail: &WorkTrail) -> &Task {
    assert_eq!(trail.tasks.len(), 1, "one task expected: {trail:#?}");
    &trail.tasks[0]
}

fn contribution<'a>(task: &'a Task, event: &Event) -> &'a Contribution {
    let id = event.id.to_hex();
    task.contributions
        .iter()
        .find(|c| c.id == id)
        .unwrap_or_else(|| panic!("contribution {id} missing from {task:#?}"))
}

fn has_gap(trail: &WorkTrail, event: &Event, needle: &str) -> bool {
    let id = event.id.to_hex();
    trail
        .gaps
        .iter()
        .any(|g| g.id.as_deref() == Some(id.as_str()) && g.reason.contains(needle))
}

fn assert_honest(trail: &WorkTrail) {
    let text = serde_json::to_string(trail).expect("json");
    for banned in [
        "\"independent\"",
        "no dispute",
        "clean",
        "\"score\"",
        "\"rung\"",
        "inactive",
        "percent",
    ] {
        assert!(!text.contains(banned), "trail contains {banned}");
    }
    for line in &trail.coverage {
        assert!(
            !line.contains("complete"),
            "coverage claims completeness: {line}"
        );
    }
}

/// The non-code chain: request, commitment, blocked update, C1 corrected,
/// C2 (q C1) accepted, an outside opinion on C1, and ordinary praise.
struct Chain {
    relay: Relay,
    channel: Uuid,
    requester: Keys,
    contributor: Keys,
    outsider: Keys,
    request: Event,
    commitment: Event,
    update: Event,
    c1: Event,
    c2: Event,
    corrected: Event,
    accepted: Event,
    opinion: Event,
    praise: Event,
}

fn chain() -> Chain {
    let channel = Uuid::new_v4();
    let (requester, contributor, outsider) = (Keys::generate(), Keys::generate(), Keys::generate());
    let request = request(channel, &requester, T0);
    let commitment = part(&request, WorkRole::Commitment, &contributor, T0 + 10, None);
    let update = part(&request, WorkRole::Update, &contributor, T0 + 20, None);
    let c1 = part(&request, WorkRole::Submission, &contributor, T0 + 30, None);
    let corrected = label(&c1, "corrected", &requester, T0 + 40);
    let c2 = part(
        &request,
        WorkRole::Submission,
        &contributor,
        T0 + 50,
        Some(&c1),
    );
    let accepted = label(&c2, "accepted", &requester, T0 + 60);
    let opinion = label(&c1, "accepted", &outsider, T0 + 70);
    let praise = sign(
        crate::builders::build_message(
            channel,
            "looks good!",
            Some(&crate::ThreadRef {
                root_event_id: request.id,
                parent_event_id: request.id,
            }),
            &[],
            false,
            &[],
            &[],
        )
        .expect("message"),
        &requester,
        T0 + 80,
    );
    let mut relay = Relay::default();
    for e in [
        &request,
        &commitment,
        &update,
        &c1,
        &corrected,
        &c2,
        &accepted,
        &opinion,
        &praise,
    ] {
        relay.add(e);
    }
    Chain {
        relay,
        channel,
        requester,
        contributor,
        outsider,
        request,
        commitment,
        update,
        c1,
        c2,
        corrected,
        accepted,
        opinion,
        praise,
    }
}

#[test]
fn exact_revision_outcomes_keep_correction_history() {
    let c = chain();
    let run = read(
        &c.relay,
        Entry::Request(c.request.id),
        scope(c.channel, &[]),
    );
    let trail = &run.trail;
    assert_eq!(trail.policy.name, "buzz.work/v1");
    let task = only_task(trail);
    assert_eq!(task.request.id, Some(c.request.id.to_hex()));
    assert_eq!(
        task.request.requester,
        Some(c.requester.public_key().to_hex())
    );
    assert_eq!(task.request.acceptance_criteria, criteria());

    let c1 = contribution(task, &c.c1);
    let c2 = contribution(task, &c.c2);
    assert_eq!((c1.role, c2.role), (R::Submission, R::Submission));
    assert_eq!(
        c2.previous,
        Some(c.c1.id.to_hex()),
        "C2 names C1 as its previous revision"
    );
    assert_eq!(c1.signer, c.contributor.public_key().to_hex());

    // C1 keeps its correction; the acceptance of C2 never moves to C1, and
    // the outsider's acceptance of C1 is an opinion, not task state.
    let c1_claims: Vec<(&str, A)> = c1
        .outcomes
        .iter()
        .map(|o| (o.claim, o.authority.status))
        .collect();
    assert_eq!(
        c1_claims,
        vec![("corrected", A::Authorized), ("accepted", A::Opinion)]
    );
    assert_eq!(c1.outcomes[0].id, c.corrected.id.to_hex());
    assert_eq!(c1.outcomes[1].id, c.opinion.id.to_hex());
    assert_eq!(c1.outcomes[1].signer, c.outsider.public_key().to_hex());
    let c2_claims: Vec<(&str, A)> = c2
        .outcomes
        .iter()
        .map(|o| (o.claim, o.authority.status))
        .collect();
    assert_eq!(c2_claims, vec![("accepted", A::Authorized)]);
    assert_eq!(c2.outcomes[0].id, c.accepted.id.to_hex());
    assert!(c2
        .outcomes
        .iter()
        .all(|o| o.scope == OutcomeScope::Revision));
    // Unrelated keys are relationship-unknown, never independent.
    assert_eq!(c2.outcomes[0].relationship, Independence::Unknown);

    // Ordinary praise is conversation, not acceptance.
    assert_eq!(task.conversation.len(), 1);
    assert_eq!(task.conversation[0].id, c.praise.id.to_hex());
    assert!(task.unbound.is_empty());
    assert_honest(trail);

    // The readable view names every source and keeps every qualifier.
    let text = trail.readable();
    for e in [
        &c.request,
        &c.commitment,
        &c.update,
        &c.c1,
        &c.c2,
        &c.corrected,
        &c.accepted,
        &c.opinion,
        &c.praise,
    ] {
        assert!(text.contains(&e.id.to_hex()), "{} missing:\n{text}", e.id);
    }
    for needle in [
        "task: Draft the Q3 summary (writing)",
        "criterion: covers the three regions",
        "state: blocked",
        &format!("follows: {}", c.c1.id),
        "an attributed opinion, not task state",
        &format!("outcome corrected {}", c.corrected.id),
        "relationship unknown",
        "conversation (not work, not acceptance)",
        "content is not fetched",
        "coverage:",
    ] {
        assert!(text.contains(needle), "{needle:?} missing:\n{text}");
    }
    assert_readable_honest(&text);
}

fn assert_readable_honest(text: &str) {
    let lower = text.to_lowercase();
    for banned in [
        "independent",
        "verified work",
        "no dispute",
        "clean",
        "score",
        "rung",
        "complete",
    ] {
        assert!(
            !lower.contains(banned),
            "readable view says {banned}:\n{text}"
        );
    }
}

#[test]
fn progress_is_signed_claims_not_inferred_state() {
    let c = chain();
    let mut relay = Relay::default();
    for e in [&c.request, &c.commitment, &c.update] {
        relay.add(e);
    }
    let run = read(&relay, Entry::Request(c.request.id), scope(c.channel, &[]));
    let task = only_task(&run.trail);
    let roles: Vec<R> = task.contributions.iter().map(|x| x.role).collect();
    assert_eq!(roles, vec![R::Commitment, R::Update]);
    assert_eq!(
        contribution(task, &c.update).state.as_deref(),
        Some("blocked")
    );
    assert!(task.contributions.iter().all(|x| x.outcomes.is_empty()));
    assert!(task.unbound.is_empty());
    assert_honest(&run.trail);
}

#[test]
fn every_entry_reaches_the_same_task() {
    let c = chain();
    let by_request = read(
        &c.relay,
        Entry::Request(c.request.id),
        scope(c.channel, &[]),
    );
    let by_contribution = read(
        &c.relay,
        Entry::Contribution(c.c2.id),
        scope(c.channel, &[]),
    );
    let by_actor = read(
        &c.relay,
        Entry::Actor(c.contributor.public_key()),
        scope(c.channel, &[]),
    );
    let want = &by_request.trail.tasks;
    assert_eq!(&by_contribution.trail.tasks, want);
    assert_eq!(&by_actor.trail.tasks, want);
}

#[test]
fn same_events_and_scope_reproduce_the_view() {
    let c = chain();
    let first = read(
        &c.relay,
        Entry::Request(c.request.id),
        scope(c.channel, &[]),
    );
    let reversed = Relay {
        events: c.relay.events.iter().rev().cloned().collect(),
        ..Relay::default()
    };
    let second = read(
        &reversed,
        Entry::Request(c.request.id),
        scope(c.channel, &[]),
    );
    assert_eq!(first.trail, second.trail);
}

#[test]
fn self_and_same_owner_judgments_stay_visible_and_qualified() {
    let channel = Uuid::new_v4();
    let (owner, agent) = (Keys::generate(), Keys::generate());
    let req = request(channel, &owner, T0);
    let own = part(&req, WorkRole::Submission, &owner, T0 + 1, None);
    let spec = PartSpec {
        summary: "agent draft".into(),
        artifact: Some("inline: draft text".into()),
        artifact_revision: Some("sha256:abc".into()),
        ..PartSpec::default()
    };
    let by_agent = sign(
        build_work_part(&req, WorkRole::Submission, &spec, None)
            .expect("part")
            .tag(auth(&owner, &agent)),
        &agent,
        T0 + 2,
    );
    let mut relay = Relay::default();
    for e in [
        &req,
        &own,
        &by_agent,
        &label(&own, "accepted", &owner, T0 + 3),
        &label(&by_agent, "accepted", &owner, T0 + 4),
    ] {
        relay.add(e);
    }
    let run = read(&relay, Entry::Request(req.id), scope(channel, &[]));
    let task = only_task(&run.trail);
    let own = contribution(task, &own);
    assert_eq!(own.outcomes[0].authority.status, A::Authorized);
    assert_eq!(own.outcomes[0].relationship, Independence::SameSigner);
    let agent_work = contribution(task, &by_agent);
    // Credit stays with the signing agent; the owner link is shown, not
    // turned into the owner's authorship.
    assert_eq!(agent_work.signer, agent.public_key().to_hex());
    assert_eq!(
        agent_work.owner_link,
        OwnerLink::Verified(owner.public_key().to_hex())
    );
    assert_eq!(agent_work.outcomes[0].authority.status, A::Authorized);
    assert_eq!(agent_work.outcomes[0].relationship, Independence::SameOwner);
    assert_honest(&run.trail);
}

#[test]
fn tampered_unreadable_and_unauthorized_evidence_supports_nothing() {
    let c = chain();
    let mut relay = Relay::default();
    for e in [&c.request, &c.commitment, &c.corrected, &c.accepted] {
        relay.add(e);
    }
    relay.tampered(&c.c1);
    // c2 is not served to this viewer.
    let run = read(
        &relay,
        Entry::Actor(c.contributor.public_key()),
        scope(c.channel, &[]),
    );
    let trail = &run.trail;
    let task = only_task(trail);
    assert_eq!(
        task.contributions.len(),
        1,
        "only the commitment is evidence: {task:#?}"
    );
    assert!(has_gap(trail, &c.c1, "does not verify"));
    // Labels found by subject key name targets that are unknown or invalid.
    assert!(has_gap(trail, &c.accepted, "was not served; unknown"));
    assert!(has_gap(
        trail,
        &c.corrected,
        "is not a contribution in this trail"
    ));
    assert_honest(trail);
}

#[test]
fn unavailable_request_leaves_requester_unknown_and_reveals_nothing() {
    let c = chain();
    let mut relay = Relay::default();
    for e in [&c.c1, &c.corrected] {
        relay.add(e);
    }
    let run = read(&relay, Entry::Contribution(c.c1.id), scope(c.channel, &[]));
    let task = only_task(&run.trail);
    assert_eq!(
        task.request.status,
        "request unavailable to this viewer; requester unknown"
    );
    assert_eq!(
        (task.request.title.as_ref(), task.request.requester.as_ref()),
        (None, None)
    );
    let outcome = &contribution(task, &c.c1).outcomes[0];
    assert_eq!(
        outcome.authority.status,
        A::Unknown,
        "authority cannot be established"
    );
}

#[test]
fn malformed_unknown_and_misdirected_messages_are_not_work() {
    let c = chain();
    let mut relay = Relay::default();
    let v2 = sign(
        EventBuilder::new(
            Kind::Custom(9),
            r#"{"summary":"x","artifact":"y","artifact_revision":"z"}"#,
        )
        .tags([
            Tag::parse(["h", &c.channel.to_string()]).expect("h"),
            Tag::parse(["e", &c.request.id.to_hex(), "", "root"]).expect("e"),
            Tag::parse(["e", &c.request.id.to_hex(), "", "reply"]).expect("e"),
            Tag::parse(["p", &c.requester.public_key().to_hex()]).expect("p"),
            Tag::parse(["t", "buzz.work/submission/v2"]).expect("t"),
        ]),
        &c.contributor,
        T0 + 31,
    );
    let wrong_channel = sign(
        EventBuilder::new(
            Kind::Custom(1985),
            r#"{"reason":"r","criteria_checked":["c"],"evidence_refs":[]}"#,
        )
        .tags([
            Tag::parse(["L", "buzz.wot"]).expect("L"),
            Tag::parse(["l", "accepted", "buzz.wot"]).expect("l"),
            Tag::parse(["e", &c.c1.id.to_hex(), ORIGIN]).expect("e"),
            Tag::parse(["p", &c.contributor.public_key().to_hex()]).expect("p"),
            Tag::parse(["h", &Uuid::new_v4().to_string()]).expect("h"),
        ]),
        &c.requester,
        T0 + 41,
    );
    let on_commitment = sign(
        EventBuilder::new(
            Kind::Custom(1985),
            r#"{"reason":"r","criteria_checked":["c"],"evidence_refs":[]}"#,
        )
        .tags([
            Tag::parse(["L", "buzz.wot"]).expect("L"),
            Tag::parse(["l", "accepted", "buzz.wot"]).expect("l"),
            Tag::parse(["e", &c.commitment.id.to_hex(), ORIGIN]).expect("e"),
            Tag::parse(["p", &c.contributor.public_key().to_hex()]).expect("p"),
            Tag::parse(["h", &c.channel.to_string()]).expect("h"),
        ]),
        &c.requester,
        T0 + 42,
    );
    for e in [
        &c.request,
        &c.commitment,
        &c.c1,
        &v2,
        &wrong_channel,
        &on_commitment,
    ] {
        relay.add(e);
    }
    let run = read(&relay, Entry::Request(c.request.id), scope(c.channel, &[]));
    let trail = &run.trail;
    let task = only_task(trail);
    assert_eq!(
        task.contributions.len(),
        2,
        "v2 is not read as work: {task:#?}"
    );
    assert!(task
        .conversation
        .iter()
        .any(|s| s.id == v2.id.to_hex() && s.note.contains("not a known role")));
    assert!(contribution(task, &c.c1).outcomes.is_empty());
    assert!(contribution(task, &c.commitment).outcomes.is_empty());
    assert!(has_gap(trail, &wrong_channel, "h does not match"));
    assert!(has_gap(
        trail,
        &on_commitment,
        "judges a commitment or update"
    ));

    // Writers refuse to build what readers would not interpret.
    assert!(build_work_part(&c.praise, WorkRole::Submission, &PartSpec::default(), None).is_err());
    assert!(build_work_label(&c.commitment, ORIGIN, "accepted", "r", &criteria(), &[]).is_err());
    assert!(build_work_label(&c.c1, ORIGIN, "great", "r", &criteria(), &[]).is_err());
    assert!(build_work_label(&c.c1, ORIGIN, "accepted", " ", &criteria(), &[]).is_err());
    assert!(build_work_part(
        &c.request,
        WorkRole::Commitment,
        &PartSpec::default(),
        Some(&c.c1)
    )
    .is_err());
}

/// LIMIT is applied before the relay's post-filter: 501 newer untyped
/// messages by the actor cannot hide an older typed contribution.
#[test]
fn limit_before_postfilter_never_hides_typed_work() {
    let c = chain();
    let mut relay = Relay::default();
    for e in [&c.request, &c.commitment] {
        relay.add(e);
    }
    for i in 0..=QUERY_LIMIT as u64 {
        let msg = crate::builders::build_message(
            c.channel,
            &format!("chatter {i}"),
            None,
            &[],
            false,
            &[],
            &[],
        )
        .expect("message");
        relay.add(&sign(msg, &c.contributor, T0 + 100 + i));
    }
    let run = read(
        &relay,
        Entry::Actor(c.contributor.public_key()),
        scope(c.channel, &[]),
    );
    let task = only_task(&run.trail);
    assert_eq!(task.contributions[0].id, c.commitment.id.to_hex());
    // No query asked the relay to apply a post-limit constraint.
    for f in &run.filters {
        for (tag, values) in &f.generic_tags {
            let pushable = !tag.uppercase
                && (matches!(tag.character, Alphabet::H | Alphabet::E | Alphabet::D)
                    || (tag.character == Alphabet::P && values.len() == 1));
            assert!(pushable, "sent a post-limit constraint {tag:?} in {f:?}");
        }
    }
    let seed = run
        .trail
        .records
        .iter()
        .find(|r| {
            r.local
                .as_ref()
                .is_some_and(|l| l.to_string().contains("buzz.work"))
        })
        .expect("typed-work seed record");
    assert_eq!(seed.pages, 2);
    assert_eq!(seed.unmatched, QUERY_LIMIT + 1);
    assert!(seed.incomplete.is_none());
    assert_honest(&run.trail);
}

#[test]
fn same_second_pages_and_budget_are_not_exhaustive() {
    let c = chain();
    let mut tie = Relay::default();
    for i in 0..QUERY_LIMIT {
        let msg = crate::builders::build_message(
            c.channel,
            &format!("tie {i}"),
            None,
            &[],
            false,
            &[],
            &[],
        )
        .expect("message");
        tie.add(&sign(msg, &c.contributor, T0 + 5));
    }
    let run = read(
        &tie,
        Entry::Actor(c.contributor.public_key()),
        scope(c.channel, &[]),
    );
    assert!(
        run.trail
            .coverage
            .iter()
            .any(|l| l.starts_with("not exhaustive: at least 500 events share second")),
        "{:#?}",
        run.trail.coverage
    );

    let mut many = Relay::default();
    for i in 0..(QUERY_LIMIT * MAX_PAGES + 1) as u64 {
        let msg = crate::builders::build_message(c.channel, "x", None, &[], false, &[], &[])
            .expect("message");
        many.add(&sign(msg, &c.contributor, T0 + i));
    }
    let run = read(
        &many,
        Entry::Actor(c.contributor.public_key()),
        scope(c.channel, &[]),
    );
    assert!(run
        .trail
        .coverage
        .iter()
        .any(|l| l.starts_with("not exhaustive: page budget")));

    let empty = read(
        &Relay::default(),
        Entry::Actor(c.contributor.public_key()),
        scope(c.channel, &[]),
    );
    assert!(empty.trail.tasks.is_empty());
    assert!(empty
        .trail
        .coverage
        .iter()
        .any(|l| l.starts_with("none found in scope")));
    assert_honest(&empty.trail);
}

#[test]
fn undeclared_scope_is_reported_not_assumed_empty() {
    let c = chain();
    let mut s = scope(c.channel, &[]);
    s.channels.clear();
    let run = read(&c.relay, Entry::Actor(c.contributor.public_key()), s);
    assert!(run
        .trail
        .gaps
        .iter()
        .any(|g| g.reason.starts_with("no channels declared")));
    assert!(run
        .trail
        .gaps
        .iter()
        .any(|g| g.reason.starts_with("no repositories declared")));
}

/// A repository, a PR R1 with updates R2 and R3, and their statuses.
struct Code {
    relay: Relay,
    coord: String,
    owner: Keys,
    author: Keys,
    r1: Event,
    r2: Event,
    r3: Event,
}

fn commit(n: u8) -> String {
    format!("{n:02x}").repeat(20)
}

fn repo(owner: &Keys, d: &str) -> GitRepoCoord {
    GitRepoCoord {
        owner: owner.public_key().to_hex(),
        id: d.into(),
    }
}

fn pr(owner: &Keys, author: &Keys, at: u64) -> Event {
    let meta = GitPullRequestMeta {
        subject: "Fix the parser".into(),
        commit: commit(1),
        clone_urls: vec!["https://git.example/r.git".into()],
        ..GitPullRequestMeta::default()
    };
    sign(
        build_git_pull_request(&repo(owner, "r"), "", &meta).expect("pr"),
        author,
        at,
    )
}

fn update(
    repo: &GitRepoCoord,
    root: &Event,
    pr_author: &Keys,
    signer: &Keys,
    n: u8,
    at: u64,
) -> Event {
    let meta = GitPrUpdateMeta {
        pr_event: root.id.to_hex(),
        pr_author: pr_author.public_key().to_hex(),
        commit: commit(n),
        clone_urls: vec!["https://git.example/r.git".into()],
        ..GitPrUpdateMeta::default()
    };
    sign(
        build_git_pr_update(repo, "", &meta).expect("update"),
        signer,
        at,
    )
}

fn status(
    kind: GitStatus,
    root: &Event,
    revision: Option<&Event>,
    merge: Option<u8>,
    keys: &Keys,
    at: u64,
) -> Event {
    let meta = GitStatusMeta {
        root_event: root.id.to_hex(),
        accepted_revision_root: revision.map(|r| r.id.to_hex()),
        merge_commit: merge.map(commit),
        ..GitStatusMeta::default()
    };
    sign(build_git_status(kind, "", &meta).expect("status"), keys, at)
}

fn code(owner: Keys, author: Keys) -> Code {
    let coord = format!("30617:{}:r", owner.public_key().to_hex());
    let announcement = sign(
        EventBuilder::new(Kind::Custom(30617), "").tags([
            Tag::parse(["d", "r"]).expect("d"),
            Tag::parse(["maintainers", &owner.public_key().to_hex()]).expect("m"),
        ]),
        &owner,
        T0 - 100,
    );
    let r1 = pr(&owner, &author, T0);
    let r2 = update(&repo(&owner, "r"), &r1, &author, &author, 2, T0 + 10);
    let r3 = update(&repo(&owner, "r"), &r1, &author, &author, 3, T0 + 20);
    let mut relay = Relay::default();
    for e in [&announcement, &r1, &r2, &r3] {
        relay.add(e);
    }
    Code {
        relay,
        coord,
        owner,
        author,
        r1,
        r2,
        r3,
    }
}

fn code_read(code: &Code, entry: Entry) -> WorkTrail {
    read(
        &code.relay,
        entry,
        scope(Uuid::new_v4(), std::slice::from_ref(&code.coord)),
    )
    .trail
}

#[test]
fn root_only_merge_stays_revision_unknown_and_ref_check_is_separate() {
    let mut c = code(Keys::generate(), Keys::generate());
    let merge = status(
        GitStatus::AppliedOrResolved,
        &c.r1,
        None,
        Some(3),
        &c.owner,
        T0 + 30,
    );
    let refs = sign(
        EventBuilder::new(Kind::Custom(30618), "").tags([
            Tag::parse(["d", "r"]).expect("d"),
            Tag::parse(["HEAD", "ref: refs/heads/main"]).expect("HEAD"),
            Tag::parse(["refs/heads/main", &commit(3)]).expect("ref"),
        ]),
        &c.owner,
        T0 + 31,
    );
    c.relay.add(&merge);
    c.relay.add(&refs);
    let trail = code_read(&c, Entry::Request(c.r1.id));
    let task = only_task(&trail);
    assert_eq!(task.request.status, "request not linked");
    let roles: Vec<R> = task.contributions.iter().map(|x| x.role).collect();
    assert_eq!(
        roles,
        vec![R::Proposal, R::ProposalUpdate, R::ProposalUpdate]
    );
    assert!(task
        .contributions
        .iter()
        .all(|x| x.unresolved.is_none() && x.outcomes.is_empty()));
    assert_eq!(contribution(task, &c.r2).revision, Some(commit(2)));
    let [outcome] = task.unbound.as_slice() else {
        panic!("one unbound outcome: {task:#?}")
    };
    assert_eq!(outcome.scope, OutcomeScope::PullRequest);
    assert!(outcome
        .note
        .as_deref()
        .is_some_and(|n| n.starts_with("PR-level merge claim; judged revision unknown")));
    assert_eq!(outcome.authority.status, A::Authorized);
    assert_eq!(
        outcome.ref_state.as_ref().map(|r| r.status),
        Some(RefStatus::Corroborated)
    );
    assert_honest(&trail);
    let text = trail.readable();
    for needle in [
        "request: request not linked",
        "outcomes that name no exact revision:",
        "PR-level merge claim; judged revision unknown",
        "merge commit corroborated by ref state",
        "not review",
        &merge.id.to_hex(),
    ] {
        assert!(text.contains(needle), "{needle:?} missing:\n{text}");
    }
    assert_readable_honest(&text);
}

#[test]
fn root_only_status_is_revision_unknown_even_with_only_r1() {
    let owner = Keys::generate();
    let author = Keys::generate();
    let r1 = pr(&owner, &author, T0);
    let mut relay = Relay::default();
    relay.add(&r1);
    relay.add(&status(
        GitStatus::AppliedOrResolved,
        &r1,
        None,
        None,
        &owner,
        T0 + 1,
    ));
    let trail = read(&relay, Entry::Request(r1.id), scope(Uuid::new_v4(), &[])).trail;
    let task = only_task(&trail);
    assert!(task.contributions[0].outcomes.is_empty());
    assert_eq!(task.unbound[0].scope, OutcomeScope::PullRequest);
}

#[test]
fn explicit_revision_binds_only_that_revision() {
    let mut c = code(Keys::generate(), Keys::generate());
    let on_r2 = status(
        GitStatus::AppliedOrResolved,
        &c.r1,
        Some(&c.r2),
        Some(2),
        &c.owner,
        T0 + 30,
    );
    let on_r1 = status(
        GitStatus::Closed,
        &c.r1,
        Some(&c.r1),
        None,
        &c.author,
        T0 + 40,
    );
    c.relay.add(&on_r2);
    c.relay.add(&on_r1);
    let trail = code_read(&c, Entry::Contribution(c.r3.id));
    let task = only_task(&trail);
    let r2 = contribution(task, &c.r2);
    assert_eq!(r2.outcomes.len(), 1);
    assert_eq!(r2.outcomes[0].id, on_r2.id.to_hex());
    assert_eq!(r2.outcomes[0].scope, OutcomeScope::Revision);
    assert_eq!(r2.outcomes[0].relationship, Independence::Unknown);
    assert!(
        contribution(task, &c.r3).outcomes.is_empty(),
        "R3 never inherits R2's outcome"
    );
    let r1 = contribution(task, &c.r1);
    assert_eq!(r1.outcomes[0].claim, "closed");
    assert_eq!(r1.outcomes[0].relationship, Independence::SameSigner);
    assert!(task.unbound.is_empty());
}

#[test]
fn self_merge_is_delivery_evidence_with_its_qualifier() {
    let author = Keys::generate();
    let c = {
        let mut c = code(author.clone(), author.clone());
        let merge = status(
            GitStatus::AppliedOrResolved,
            &c.r1,
            Some(&c.r1),
            None,
            &author,
            T0 + 30,
        );
        c.relay.add(&merge);
        c
    };
    let trail = code_read(&c, Entry::Request(c.r1.id));
    let r1 = contribution(only_task(&trail), &c.r1);
    assert_eq!(r1.outcomes[0].claim, "merged");
    assert_eq!(r1.outcomes[0].authority.status, A::Authorized);
    assert_eq!(r1.outcomes[0].relationship, Independence::SameSigner);
    assert_honest(&trail);
}

#[test]
fn invalid_update_links_stay_unresolved_and_never_bind() {
    let mut c = code(Keys::generate(), Keys::generate());
    let stranger = Keys::generate();
    let by_stranger = update(
        &repo(&c.owner, "r"),
        &c.r1,
        &c.author,
        &stranger,
        4,
        T0 + 21,
    );
    let wrong_p = update(
        &repo(&c.owner, "r"),
        &c.r1,
        &stranger,
        &c.author,
        5,
        T0 + 22,
    );
    let wrong_repo = update(
        &repo(&c.owner, "other"),
        &c.r1,
        &c.author,
        &c.author,
        6,
        T0 + 23,
    );
    let two_tips = {
        let meta = GitPrUpdateMeta {
            pr_event: c.r1.id.to_hex(),
            pr_author: c.author.public_key().to_hex(),
            commit: commit(7),
            clone_urls: vec!["https://git.example/r.git".into()],
            ..GitPrUpdateMeta::default()
        };
        let b = build_git_pr_update(&repo(&c.owner, "r"), "", &meta)
            .expect("update")
            .tag(Tag::parse(["c", &commit(8)]).expect("c"));
        sign(b, &c.author, T0 + 24)
    };
    let on_stranger = status(
        GitStatus::AppliedOrResolved,
        &c.r1,
        Some(&by_stranger),
        None,
        &c.owner,
        T0 + 30,
    );
    let missing = EventId::all_zeros();
    let on_missing = {
        let meta = GitStatusMeta {
            root_event: c.r1.id.to_hex(),
            accepted_revision_root: Some(missing.to_hex()),
            ..GitStatusMeta::default()
        };
        sign(
            build_git_status(GitStatus::AppliedOrResolved, "", &meta).expect("status"),
            &c.owner,
            T0 + 31,
        )
    };
    for e in [
        &by_stranger,
        &wrong_p,
        &wrong_repo,
        &two_tips,
        &on_stranger,
        &on_missing,
    ] {
        c.relay.add(e);
    }
    let trail = code_read(&c, Entry::Request(c.r1.id));
    let task = only_task(&trail);
    let unresolved = |e: &Event| contribution(task, e).unresolved.clone().unwrap_or_default();
    assert!(unresolved(&by_stranger).contains("signed by a key other than"));
    assert!(unresolved(&wrong_p).contains("P does not name"));
    assert!(unresolved(&wrong_repo).contains("different repository"));
    assert!(unresolved(&two_tips).contains("exactly one c"));
    assert!(task.contributions.iter().all(|x| x.outcomes.is_empty()));
    let notes: BTreeSet<String> = task.unbound.iter().filter_map(|o| o.note.clone()).collect();
    assert!(
        notes
            .iter()
            .any(|n| n.contains("not a validated revision of this PR")),
        "{notes:?}"
    );
    assert!(
        notes.iter().any(|n| n.contains("which is unavailable")),
        "{notes:?}"
    );
    assert!(notes
        .iter()
        .all(|n| n.starts_with("PR-level merge claim; judged revision unknown")));
}

#[test]
fn update_of_another_pr_or_with_two_e_never_binds_here() {
    let mut c = code(Keys::generate(), Keys::generate());
    // A valid update, but of a different PR by the same author.
    let other = pr(&c.owner, &c.author, T0 + 1);
    let of_other = update(
        &repo(&c.owner, "r"),
        &other,
        &c.author,
        &c.author,
        9,
        T0 + 25,
    );
    let two_e = {
        let meta = GitPrUpdateMeta {
            pr_event: c.r1.id.to_hex(),
            pr_author: c.author.public_key().to_hex(),
            commit: commit(10),
            clone_urls: vec!["https://git.example/r.git".into()],
            ..GitPrUpdateMeta::default()
        };
        let b = build_git_pr_update(&repo(&c.owner, "r"), "", &meta)
            .expect("update")
            .tag(Tag::parse(["E", &other.id.to_hex()]).expect("E"));
        sign(b, &c.author, T0 + 26)
    };
    let on_other = status(
        GitStatus::AppliedOrResolved,
        &c.r1,
        Some(&of_other),
        None,
        &c.owner,
        T0 + 32,
    );
    let on_two_e = status(
        GitStatus::AppliedOrResolved,
        &c.r1,
        Some(&two_e),
        None,
        &c.owner,
        T0 + 33,
    );
    for e in [&other, &of_other, &two_e, &on_other, &on_two_e] {
        c.relay.add(e);
    }
    let trail = code_read(&c, Entry::Request(c.r1.id));
    let task = trail
        .tasks
        .iter()
        .find(|t| t.contributions.iter().any(|x| x.id == c.r1.id.to_hex()))
        .expect("R1's task");
    // Neither status binds to any revision of R1.
    assert!(
        task.contributions.iter().all(|x| x.outcomes.is_empty()),
        "{task:#?}"
    );
    for s in [&on_other, &on_two_e] {
        let o = task
            .unbound
            .iter()
            .find(|o| o.id == s.id.to_hex())
            .unwrap_or_else(|| panic!("{} not shown: {task:#?}", s.id));
        assert!(
            o.note
                .as_deref()
                .is_some_and(|n| n.starts_with("PR-level merge claim; judged revision unknown")),
            "{o:#?}"
        );
    }
    // The update of another PR is never a revision of R1.
    assert!(task
        .contributions
        .iter()
        .all(|x| x.id != of_other.id.to_hex()));
}

#[test]
fn saved_trail_rechecks_its_evidence_without_the_origin() {
    let c = chain();
    let mut relay = Relay::default();
    for e in [&c.request, &c.commitment, &c.c2, &c.accepted] {
        relay.add(e);
    }
    relay.tampered(&c.c1);
    let entry = Entry::Request(c.request.id);
    let first = read_at(&relay, entry, scope(c.channel, &[]), Some(T0 + 900)).trail;
    let later = read_at(&relay, entry, scope(c.channel, &[]), Some(T0 + 950)).trail;

    // Observation times are kept as evidence metadata and change nothing
    // derived from the events.
    assert_eq!(
        (&first.tasks, &first.gaps, &first.coverage),
        (&later.tasks, &later.gaps, &later.coverage)
    );
    let kept = |t: &WorkTrail, e: &Event| {
        t.evidence
            .events
            .iter()
            .find(|k| k.id == e.id.to_hex())
            .cloned()
            .unwrap_or_else(|| panic!("{} not kept", e.id))
    };
    let request = kept(&first, &c.request);
    assert!(request.valid && request.origin == ORIGIN && !request.records.is_empty());
    assert_eq!(request.observed_at, Some(T0 + 900));
    assert_eq!(kept(&later, &c.request).observed_at, Some(T0 + 950));
    assert_eq!(
        request.event,
        serde_json::to_value(&c.request).expect("json")
    );
    for r in &request.records {
        assert!(
            *r < first.records.len(),
            "record {r} is cited but not listed"
        );
    }
    // Bytes that failed verification stay kept and flagged.
    assert!(!kept(&first, &c.c1).valid);

    // The saved JSON alone rechecks: no origin is consulted.
    let mut saved = serde_json::to_value(&first).expect("json");
    let check = super::check_evidence(&saved).expect("check");
    assert!(check.holds(), "{check:#?}");
    assert_eq!(check.events, first.evidence.events.len());
    assert_eq!(check.verified, check.events - 1);

    // The recheck authenticates envelopes, not what the reader wrote around
    // them: an edited summary or authority label still passes, and every
    // result says so.
    let mut relabeled = saved.clone();
    relabeled["tasks"][0]["contributions"][0]["summary"] = Value::String("rewritten".into());
    let check = super::check_evidence(&relabeled).expect("check");
    assert!(check.holds());
    assert_eq!(check.not_checked, super::check::NOT_CHECKED);
    let unchecked = check.not_checked.join("; ");
    for needle in ["summaries", "authority", "provenance", "completeness"] {
        assert!(unchecked.contains(needle), "{needle} missing: {unchecked}");
    }

    // Editing a kept envelope is caught, and the claims it supported are
    // no longer supported.
    let c2 = c.c2.id.to_hex();
    for k in saved["evidence"]["events"].as_array_mut().expect("events") {
        if k["id"] == c2.as_str() {
            k["event"]["content"] = Value::String("edited later".into());
        }
    }
    let check = super::check_evidence(&saved).expect("check");
    assert!(!check.holds());
    assert!(
        check.failed.iter().any(|f| f.starts_with(&c2)),
        "{check:#?}"
    );
    assert!(check.unsupported.contains(&c2), "{check:#?}");
    assert!(first.readable().contains("signed events kept as served"));
    assert!(first
        .readable()
        .contains("authenticates those events offline, not this view"));
}

/// A repository with one older PR by the actor and one by someone else.
struct RepoWork {
    relay: Relay,
    coord: String,
    owner: Keys,
    actor: Keys,
    mine: Event,
    theirs: Event,
}

fn repo_work() -> RepoWork {
    let mut c = code(Keys::generate(), Keys::generate());
    let other = Keys::generate();
    let theirs = pr(&c.owner, &other, T0 + 5);
    c.relay.add(&theirs);
    RepoWork {
        relay: c.relay,
        coord: c.coord,
        owner: c.owner,
        actor: c.author,
        mine: c.r1,
        theirs,
    }
}

/// A status as the CLI writes it: repository `a` and only the owner's `p`.
fn repo_status(kind: GitStatus, w: &RepoWork, target: &Event, at: u64) -> Event {
    let meta = GitStatusMeta {
        root_event: target.id.to_hex(),
        repo: Some(repo(&w.owner, "r")),
        recipients: vec![w.owner.public_key().to_hex()],
        ..GitStatusMeta::default()
    };
    sign(
        build_git_status(kind, "", &meta).expect("status"),
        &w.owner,
        at,
    )
}

fn windowed(w: &RepoWork, since: u64) -> Scope {
    Scope {
        since: Some(since),
        ..scope(Uuid::new_v4(), std::slice::from_ref(&w.coord))
    }
}

#[test]
fn actor_trail_finds_repo_only_statuses_on_older_work() {
    let mut w = repo_work();
    let closed = repo_status(GitStatus::Closed, &w, &w.mine, T0 + 200);
    let unrelated = repo_status(GitStatus::Closed, &w, &w.theirs, T0 + 210);
    w.relay.add(&closed);
    w.relay.add(&unrelated);
    let run = read(
        &w.relay,
        Entry::Actor(w.actor.public_key()),
        windowed(&w, T0 + 100),
    );
    let trail = &run.trail;

    // The PR predates the window; the closure inside it names only the
    // repository owner, yet it is found and attributed to the PR's signer.
    let task = only_task(trail);
    let mine = contribution(task, &w.mine);
    assert_eq!(mine.signer, w.actor.public_key().to_hex());
    let [outcome] = task.unbound.as_slice() else {
        panic!("the closure: {task:#?}")
    };
    assert_eq!(outcome.id, closed.id.to_hex());
    assert_eq!(outcome.claim, "closed");
    assert_eq!(outcome.scope, OutcomeScope::PullRequest);
    assert_eq!(outcome.authority.status, A::Authorized);

    // Someone else's PR in the same repository is not the actor's work,
    // and is not reported as a gap either.
    let text = serde_json::to_string(&(&trail.tasks, &trail.gaps)).expect("json");
    for absent in [&w.theirs, &unrelated] {
        assert!(!text.contains(&absent.id.to_hex()), "{} leaked", absent.id);
    }
    assert!(trail
        .coverage
        .iter()
        .any(|l| l.contains("1630") && l.contains(&w.coord)));
    assert_honest(trail);
}

#[test]
fn repo_status_discovery_failure_is_a_coverage_gap() {
    let mut w = repo_work();
    w.relay
        .add(&repo_status(GitStatus::Closed, &w, &w.mine, T0 + 200));
    w.relay.refuse = vec![1630];
    let trail = read(
        &w.relay,
        Entry::Actor(w.actor.public_key()),
        windowed(&w, T0 + 100),
    )
    .trail;
    assert!(trail.tasks.is_empty(), "{trail:#?}");
    assert!(
        trail
            .coverage
            .iter()
            .any(|l| l.starts_with("inaccessible: restricted") && l.contains(&w.coord)),
        "{:#?}",
        trail.coverage
    );
    assert!(trail.readable().contains("inaccessible: restricted"));
    assert_honest(&trail);
}

#[test]
fn issue_is_a_request_whose_statuses_judge_no_contribution() {
    let mut w = repo_work();
    let requester = Keys::generate();
    let issue = sign(
        crate::builders::build_git_issue(
            &repo(&w.owner, "r"),
            "Parser drops trailing commas",
            "Steps to reproduce: …",
            &crate::builders::GitIssueMeta::default(),
        )
        .expect("issue"),
        &requester,
        T0 + 300,
    );
    let resolved = repo_status(GitStatus::AppliedOrResolved, &w, &issue, T0 + 400);
    w.relay.add(&issue);
    w.relay.add(&resolved);

    let by_request = read(&w.relay, Entry::Request(issue.id), windowed(&w, T0)).trail;
    let by_actor = read(
        &w.relay,
        Entry::Actor(requester.public_key()),
        windowed(&w, T0),
    )
    .trail;
    assert_eq!(by_request.tasks, by_actor.tasks);
    let task = only_task(&by_request);
    let r = &task.request;
    assert_eq!(r.id, Some(issue.id.to_hex()));
    assert_eq!(r.kind, Some(1621));
    assert_eq!(r.requester, Some(requester.public_key().to_hex()));
    assert_eq!(r.title.as_deref(), Some("Parser drops trailing commas"));
    // No pull request is linked to it, and none is inferred.
    assert!(task.contributions.is_empty(), "{task:#?}");
    let [status] = task.unbound.as_slice() else {
        panic!("one request status: {task:#?}")
    };
    assert_eq!(status.claim, "resolved");
    assert_eq!(status.scope, OutcomeScope::Request);
    assert_eq!(status.authority.status, A::Authorized);
    assert!(status
        .note
        .as_deref()
        .is_some_and(|n| n.contains("judges no contribution")));
    assert!(by_request
        .gaps
        .iter()
        .all(|g| !g.reason.contains("not typed work")));
    let text = by_request.readable();
    for needle in [
        "task: Parser drops trailing commas",
        "code issue; its signer is the requester",
        "none linked: no convention links pull requests to issues yet",
        "request statuses (judge no contribution):",
        &resolved.id.to_hex(),
    ] {
        assert!(text.contains(needle), "{needle:?} missing:\n{text}");
    }
    assert!(
        !text.contains(&w.mine.id.to_hex()),
        "an unlinked PR appeared"
    );
    assert_readable_honest(&text);
    assert_honest(&by_request);
}

// Counterexamples from Cobra's re-review of 800dd72be7.

#[test]
fn a_status_cannot_pull_actor_work_in_from_an_undeclared_repo() {
    let mut w = repo_work();
    let outside_coord = repo(&w.owner, "outside-declared-scope");
    let outside = sign(
        build_git_pull_request(
            &outside_coord,
            "",
            &GitPullRequestMeta {
                subject: "Outside the declared repository".into(),
                commit: commit(7),
                clone_urls: vec!["https://git.example/outside.git".into()],
                ..GitPullRequestMeta::default()
            },
        )
        .expect("pr"),
        &w.actor,
        T0 + 1,
    );
    // Tagged with the declared repository, but its target is the actor's
    // older PR in another one. A signature does not make the link valid.
    let mismatched = repo_status(GitStatus::Closed, &w, &outside, T0 + 200);
    w.relay.add(&outside);
    w.relay.add(&mismatched);
    let trail = read(
        &w.relay,
        Entry::Actor(w.actor.public_key()),
        windowed(&w, T0 + 100),
    )
    .trail;
    assert!(
        trail.tasks.is_empty(),
        "outside work became actor work: {trail:#?}"
    );
    assert!(
        has_gap(&trail, &mismatched, "but its target"),
        "{:#?}",
        trail.gaps
    );
    assert_honest(&trail);
}

/// A merge status on R1 judging R2, plus one extra tag.
fn merge_with(c: &Code, extra: Tag, at: u64) -> Event {
    let meta = GitStatusMeta {
        root_event: c.r1.id.to_hex(),
        accepted_revision_root: Some(c.r2.id.to_hex()),
        repo: Some(repo(&c.owner, "r")),
        ..GitStatusMeta::default()
    };
    let b = build_git_status(GitStatus::AppliedOrResolved, "", &meta).expect("status");
    sign(b.tag(extra), &c.owner, at)
}

#[test]
fn a_malformed_second_reply_leaves_the_revision_unknown() {
    let mut c = code(Keys::generate(), Keys::generate());
    let ambiguous = merge_with(
        &c,
        Tag::parse(["e", "not-an-event-id", "", "reply"]).expect("e"),
        T0 + 30,
    );
    c.relay.add(&ambiguous);
    let trail = code_read(&c, Entry::Request(c.r1.id));
    let task = only_task(&trail);
    assert!(
        task.contributions.iter().all(|x| x.outcomes.is_empty()),
        "ambiguous status binds: {task:#?}"
    );
    let [outcome] = task.unbound.as_slice() else {
        panic!("one unbound outcome: {task:#?}")
    };
    assert_eq!(outcome.id, ambiguous.id.to_hex());
    assert!(outcome.note.as_deref().is_some_and(|n| n
        .starts_with("PR-level merge claim; judged revision unknown")
        && n.contains("more than one revision")));
}

#[test]
fn several_roots_or_repositories_are_not_interpreted() {
    let mut c = code(Keys::generate(), Keys::generate());
    let other = EventId::from_slice(&[8; 32]).expect("id").to_hex();
    let two_roots = merge_with(
        &c,
        Tag::parse(["e", &other, "", "root"]).expect("e"),
        T0 + 30,
    );
    let another = format!("30617:{}:another", c.owner.public_key());
    let two_repos = merge_with(&c, Tag::parse(["a", &another]).expect("a"), T0 + 31);
    c.relay.add(&two_roots);
    c.relay.add(&two_repos);
    let trail = code_read(&c, Entry::Request(c.r1.id));
    let task = only_task(&trail);
    assert!(
        task.contributions.iter().all(|x| x.outcomes.is_empty()) && task.unbound.is_empty(),
        "contradictory statuses were interpreted: {task:#?}"
    );
    assert!(
        has_gap(&trail, &two_roots, "names 2 roots"),
        "{:#?}",
        trail.gaps
    );
    assert!(
        has_gap(&trail, &two_repos, "names 2 different repositories"),
        "{:#?}",
        trail.gaps
    );
    assert_honest(&trail);
}
