use buzz_core::bw::{Evidence, Trust};
use buzz_sdk::bw::operations::{
    accept_to_backlog, assign_writer, enroll_issue, mark_implemented, move_to_ready,
    start_development, AcceptToBacklogRequest, AssignWriterRequest, IssueRequest,
    MarkImplementedRequest, MoveToReadyRequest, OperationPort, PortFuture, RawHistory,
    SigningIdentity,
};
use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, Tag, Timestamp};
use serde_json::Value;
use std::{
    collections::VecDeque,
    future::Future,
    task::{Context, Poll, Waker},
};

struct FakePort {
    fixtures: Value,
    labels: Vec<&'static str>,
    stored: Vec<Event>,
    pending: Option<Event>,
    times: VecDeque<u64>,
    keys: Keys,
    missing_readbacks: usize,
    submissions: Vec<String>,
    submitted_bytes: Vec<String>,
}

impl FakePort {
    fn from_history(
        labels: Vec<&'static str>,
        times: impl IntoIterator<Item = u64>,
        key_number: u8,
    ) -> Self {
        Self {
            fixtures: serde_json::from_str(include_str!("../../../docs/nips/NIP-BW.fixtures.json"))
                .expect("fixtures"),
            labels,
            stored: vec![],
            pending: None,
            times: times.into_iter().collect(),
            keys: Keys::parse(&format!("{key_number:064x}")).expect("fixture key"),
            missing_readbacks: 0,
            submissions: vec![],
            submitted_bytes: vec![],
        }
    }

    fn enrollment() -> Self {
        let mut port = Self::from_history(vec!["repo", "policy", "root_a"], [1_800_000_002], 2);
        port.missing_readbacks = 1;
        port
    }

    fn trust_and_evidence(&self) -> (Trust, Evidence, u64) {
        let case = self.fixtures["cases"]
            .as_array()
            .expect("cases")
            .iter()
            .find(|case| case["name"] == "issue-state-positive")
            .expect("issue-state-positive");
        (
            serde_json::from_value(case["trust"].clone()).expect("trust"),
            serde_json::from_value(case["external"].clone()).expect("evidence"),
            case["now"].as_u64().expect("now"),
        )
    }
}

impl OperationPort for FakePort {
    type Error = String;

    fn now(&mut self) -> Result<u64, Self::Error> {
        self.times
            .pop_front()
            .ok_or_else(|| "unexpected clock read".to_string())
    }

    fn load_history<'a>(&'a mut self, _repo: &'a str) -> PortFuture<'a, RawHistory, Self::Error> {
        let (trust, evidence, now) = self.trust_and_evidence();
        let mut events = self
            .labels
            .iter()
            .map(|label| {
                serde_json::to_vec(&self.fixtures["events"][label]["event"]).expect("fixture event")
            })
            .collect::<Vec<_>>();
        events.extend(self.stored.iter().map(|event| event.as_json().into_bytes()));
        Box::pin(async move {
            Ok(RawHistory {
                trust,
                evidence,
                now,
                events,
            })
        })
    }

    fn signing_identity<'a>(
        &'a mut self,
        required_pubkey: Option<&'a str>,
    ) -> PortFuture<'a, SigningIdentity, Self::Error> {
        let pubkey = self.keys.public_key().to_hex();
        Box::pin(async move {
            if required_pubkey.is_some_and(|required| required != pubkey) {
                return Err("wrong signer".to_string());
            }
            Ok(SigningIdentity {
                pubkey,
                event_tags: vec![],
            })
        })
    }

    fn sign<'a>(
        &'a mut self,
        candidate: &'a Value,
        _required_pubkey: Option<&'a str>,
    ) -> PortFuture<'a, Event, Self::Error> {
        Box::pin(async move {
            let tags = candidate["tags"]
                .as_array()
                .ok_or("missing tags")?
                .iter()
                .map(|tag| {
                    let parts = tag
                        .as_array()
                        .ok_or("bad tag")?
                        .iter()
                        .map(|part| part.as_str().ok_or("bad tag part").map(str::to_owned))
                        .collect::<Result<Vec<_>, _>>()?;
                    Tag::parse(parts).map_err(|error| error.to_string())
                })
                .collect::<Result<Vec<_>, _>>()?;
            let kind = match candidate["kind"].as_u64().ok_or("missing kind")? {
                1 => Kind::TextNote,
                value => Kind::Custom(value as u16),
            };
            EventBuilder::new(
                kind,
                candidate["content"].as_str().ok_or("missing content")?,
            )
            .tags(tags)
            .custom_created_at(Timestamp::from_secs(
                candidate["created_at"].as_u64().ok_or("missing time")?,
            ))
            .sign_with_keys(&self.keys)
            .map_err(|error| error.to_string())
        })
    }

    fn submit<'a>(&'a mut self, event: &'a Event) -> PortFuture<'a, (), Self::Error> {
        self.submissions.push(event.id.to_hex());
        self.submitted_bytes.push(event.as_json());
        self.pending = Some(event.clone());
        Box::pin(async { Ok(()) })
    }

    fn readback<'a>(&'a mut self, id: &'a str) -> PortFuture<'a, Option<Event>, Self::Error> {
        Box::pin(async move {
            if self.missing_readbacks > 0 {
                self.missing_readbacks -= 1;
                return Ok(None);
            }
            let event = self.pending.clone().filter(|event| event.id.to_hex() == id);
            if let Some(event) = &event {
                if !self.stored.iter().any(|stored| stored.id == event.id) {
                    self.stored.push(event.clone());
                }
            }
            Ok(event)
        })
    }

    fn remote_head<'a>(
        &'a mut self,
        _repo: &'a str,
        _stream: &'a str,
    ) -> PortFuture<'a, String, Self::Error> {
        Box::pin(async { Ok("1".repeat(40)) })
    }
}

fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = Box::pin(future);
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}

#[test]
fn enrollment_retries_the_same_signed_bytes_once_then_becomes_idempotent() {
    let mut port = FakePort::enrollment();
    let repo = port.trust_and_evidence().0.repo;
    let issue = port.fixtures["events"]["root_a"]["event"]["id"]
        .as_str()
        .expect("issue")
        .to_string();

    let first = block_on(enroll_issue(
        &mut port,
        IssueRequest::new(repo.clone(), issue.clone()),
    ))
    .expect("enrollment");
    assert!(!first.already_completed);
    assert_eq!(first.steps[0].status.as_str(), "published");
    assert_eq!(port.submissions.len(), 2);
    assert!(port
        .submissions
        .iter()
        .all(|event_id| event_id == &port.submissions[0]));
    assert_eq!(port.submitted_bytes[0], port.submitted_bytes[1]);

    let submitted_before_retry = port.submissions.len();
    let retry = block_on(enroll_issue(&mut port, IssueRequest::new(repo, issue)))
        .expect("idempotent retry");
    assert!(retry.already_completed);
    assert_eq!(retry.steps[0].status.as_str(), "reused");
    assert_eq!(port.submissions.len(), submitted_before_retry);
}

fn coordinates(port: &FakePort) -> (String, String) {
    (
        port.trust_and_evidence().0.repo,
        port.fixtures["events"]["root_a"]["event"]["id"]
            .as_str()
            .expect("issue")
            .to_owned(),
    )
}

fn assert_wire_shape(port: &FakePort, label: &str) {
    let actual: Value =
        serde_json::from_str(&port.stored.last().expect("published event").as_json())
            .expect("actual event");
    let expected = &port.fixtures["events"][label]["event"];
    for field in ["pubkey", "created_at", "kind", "tags"] {
        assert_eq!(actual[field], expected[field], "{field}");
    }
    let actual_body: Value =
        serde_json::from_str(actual["content"].as_str().expect("actual content")).expect("body");
    let expected_body: Value =
        serde_json::from_str(expected["content"].as_str().expect("expected content"))
            .expect("body");
    assert_eq!(actual_body, expected_body);
}

#[test]
fn accept_resumes_from_the_existing_accept_and_only_publishes_backlog() {
    let mut port = FakePort::from_history(
        vec![
            "repo", "policy", "root_a", "enroll_a", "update_a", "accept_a",
        ],
        [1_800_000_005],
        1,
    );
    let (repo, issue) = coordinates(&port);
    let first = block_on(accept_to_backlog(
        &mut port,
        AcceptToBacklogRequest {
            repo: repo.clone(),
            issue: issue.clone(),
            delegated: false,
        },
    ))
    .expect("resume accept");
    assert_eq!(
        first
            .steps
            .iter()
            .map(|step| (step.name.as_str(), step.status.as_str()))
            .collect::<Vec<_>>(),
        vec![("accept", "reused"), ("backlog", "published")]
    );
    assert_eq!(port.submissions.len(), 1);
    assert_eq!(
        port.submissions[0],
        port.fixtures["events"]["backlog_a"]["event"]["id"]
            .as_str()
            .expect("backlog id")
    );

    let submissions = port.submissions.len();
    let retry = block_on(accept_to_backlog(
        &mut port,
        AcceptToBacklogRequest {
            repo,
            issue,
            delegated: false,
        },
    ))
    .expect("completed accept");
    assert!(retry.already_completed);
    assert_eq!(port.submissions.len(), submissions);
}

#[test]
fn writer_change_resumes_after_unassignment_and_rebinds_ready() {
    let mut port = FakePort::from_history(
        vec![
            "repo",
            "policy",
            "root_a",
            "enroll_a",
            "update_a",
            "accept_a",
            "backlog_a",
            "assign_a",
            "ready_a",
            "unassign",
        ],
        [1_800_000_011, 1_800_000_012],
        1,
    );
    let (repo, issue) = coordinates(&port);
    let writer = port.fixtures["events"]["reassign"]["event"]["tags"][2][1]
        .as_str()
        .expect("new writer")
        .to_owned();
    let first = block_on(assign_writer(
        &mut port,
        AssignWriterRequest {
            repo: repo.clone(),
            issue: issue.clone(),
            writer: writer.clone(),
        },
    ))
    .expect("resume writer change");
    assert_eq!(
        first
            .steps
            .iter()
            .map(|step| (step.name.as_str(), step.status.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("unassignment", "reused"),
            ("assignment", "published"),
            ("ready_rebind", "published"),
        ]
    );
    assert_eq!(port.submissions.len(), 2);

    let submissions = port.submissions.len();
    let retry = block_on(assign_writer(
        &mut port,
        AssignWriterRequest {
            repo,
            issue,
            writer,
        },
    ))
    .expect("completed writer change");
    assert!(retry.already_completed);
    assert_eq!(port.submissions.len(), submissions);
}

#[test]
fn move_to_ready_refuses_a_forked_update_chain_without_publishing() {
    let mut port = FakePort::from_history(
        vec![
            "repo",
            "policy",
            "root_a",
            "enroll_a",
            "update_a",
            "text-fork",
            "accept_a",
            "backlog_a",
            "assign_a",
        ],
        [],
        1,
    );
    let (repo, issue) = coordinates(&port);
    let error = block_on(move_to_ready(
        &mut port,
        MoveToReadyRequest {
            repo,
            issue,
            stream: "windows-integration".to_owned(),
            rework: None,
            terminal_set: None,
        },
    ))
    .expect_err("fork must fail closed");
    assert!(error.to_string().contains("bw:conflict:causality:fork"));
    assert!(port.submissions.is_empty());
}

#[test]
fn writer_lifecycle_events_match_the_pinned_wire_shapes() {
    let mut start_port = FakePort::from_history(
        vec![
            "repo",
            "policy",
            "root_a",
            "enroll_a",
            "update_a",
            "accept_a",
            "backlog_a",
            "assign_a",
            "ready_a",
        ],
        [1_800_000_008],
        5,
    );
    let (repo, issue) = coordinates(&start_port);
    let _started = block_on(start_development(
        &mut start_port,
        IssueRequest::new(repo.clone(), issue.clone()),
    ))
    .expect("start development");
    assert_wire_shape(&start_port, "dev_a");
    let start_submissions = start_port.submissions.len();
    let start_retry = block_on(start_development(
        &mut start_port,
        IssueRequest::new(repo.clone(), issue.clone()),
    ))
    .expect("reuse development");
    assert!(start_retry.already_completed);
    assert_eq!(start_port.submissions.len(), start_submissions);

    let mut implemented_port = FakePort::from_history(
        vec![
            "repo",
            "policy",
            "root_a",
            "enroll_a",
            "update_a",
            "accept_a",
            "backlog_a",
            "assign_a",
            "ready_a",
            "dev_a",
        ],
        [1_800_000_009],
        5,
    );
    let _implemented = block_on(mark_implemented(
        &mut implemented_port,
        MarkImplementedRequest {
            repo: repo.clone(),
            issue: issue.clone(),
            commit: Some("1".repeat(40)),
            tests: "Fixture only; no product test".to_owned(),
        },
    ))
    .expect("mark implemented");
    assert_wire_shape(&implemented_port, "implemented_a");
    let implemented_submissions = implemented_port.submissions.len();
    let implemented_retry = block_on(mark_implemented(
        &mut implemented_port,
        MarkImplementedRequest {
            repo,
            issue,
            commit: Some("1".repeat(40)),
            tests: "Fixture only; no product test".to_owned(),
        },
    ))
    .expect("reuse implemented");
    assert!(implemented_retry.already_completed);
    assert_eq!(implemented_port.submissions.len(), implemented_submissions);
}

#[test]
fn move_to_ready_publishes_the_pinned_shape_then_reuses_it() {
    let mut port = FakePort::from_history(
        vec![
            "repo",
            "policy",
            "root_a",
            "enroll_a",
            "update_a",
            "accept_a",
            "backlog_a",
            "assign_a",
        ],
        [1_800_000_007],
        1,
    );
    let (repo, issue) = coordinates(&port);
    let first = block_on(move_to_ready(
        &mut port,
        MoveToReadyRequest {
            repo: repo.clone(),
            issue: issue.clone(),
            stream: "windows-integration".to_owned(),
            rework: None,
            terminal_set: None,
        },
    ))
    .expect("move to ready");
    assert!(!first.already_completed);
    assert_wire_shape(&port, "ready_a");

    let submissions = port.submissions.len();
    let retry = block_on(move_to_ready(
        &mut port,
        MoveToReadyRequest {
            repo,
            issue,
            stream: "windows-integration".to_owned(),
            rework: None,
            terminal_set: None,
        },
    ))
    .expect("reuse ready");
    assert!(retry.already_completed);
    assert_eq!(retry.steps.len(), 1);
    assert_eq!(retry.steps[0].name, "ready");
    assert_eq!(port.submissions.len(), submissions);
}

#[test]
fn accept_refuses_an_ambiguous_triage_head_without_publishing() {
    let mut port = FakePort::from_history(
        vec![
            "repo",
            "policy",
            "root_a",
            "enroll_a",
            "update_a",
            "accept_a",
            "need-info",
        ],
        [],
        1,
    );
    let (repo, issue) = coordinates(&port);
    let error = block_on(accept_to_backlog(
        &mut port,
        AcceptToBacklogRequest {
            repo,
            issue,
            delegated: false,
        },
    ))
    .expect_err("ambiguous triage must fail closed");
    assert!(error.to_string().starts_with("bw:conflict:causality:"));
    assert!(port.submissions.is_empty());
}

#[test]
fn writer_change_refuses_an_ambiguous_assignment_head_without_publishing() {
    let mut port = FakePort::from_history(
        vec![
            "repo",
            "policy",
            "root_a",
            "enroll_a",
            "update_a",
            "accept_a",
            "backlog_a",
            "assign_a",
        ],
        [],
        1,
    );
    let (repo, issue) = coordinates(&port);
    let other_writer = Keys::parse(&format!("{:064x}", 6))
        .expect("other writer key")
        .public_key()
        .to_hex();
    let competing = EventBuilder::new(Kind::TextNote, "Select competing worker")
        .tags(vec![
            Tag::parse(vec!["e", &issue, "", "root"]).expect("root tag"),
            Tag::parse(vec!["a", &repo]).expect("repo tag"),
            Tag::parse(vec!["p", &other_writer]).expect("writer tag"),
            Tag::parse(vec!["t", "assignment"]).expect("operation tag"),
        ])
        .custom_created_at(Timestamp::from_secs(1_800_000_007))
        .sign_with_keys(&port.keys)
        .expect("competing assignment");
    port.stored.push(competing);

    let error = block_on(assign_writer(
        &mut port,
        AssignWriterRequest {
            repo,
            issue,
            writer: other_writer,
        },
    ))
    .expect_err("ambiguous assignment must fail closed");
    assert!(error.to_string().starts_with("bw:conflict:causality:"));
    assert!(port.submissions.is_empty());
}

#[test]
fn writer_change_refuses_an_incompatible_ready_rebind_without_publishing() {
    let mut port = FakePort::from_history(
        vec![
            "repo",
            "policy",
            "root_a",
            "enroll_a",
            "update_a",
            "accept_a",
            "backlog_a",
            "assign_a",
            "ready_a",
            "unassign",
        ],
        [1_800_000_011, 1_800_000_012],
        1,
    );
    let (repo, issue) = coordinates(&port);
    let writer = port.fixtures["events"]["reassign"]["event"]["tags"][2][1]
        .as_str()
        .expect("new writer")
        .to_owned();
    block_on(assign_writer(
        &mut port,
        AssignWriterRequest {
            repo: repo.clone(),
            issue: issue.clone(),
            writer: writer.clone(),
        },
    ))
    .expect("complete writer change fixture");

    let valid_rebind = port.stored.pop().expect("ready rebind");
    let mut candidate: Value = serde_json::from_str(&valid_rebind.as_json()).expect("ready event");
    let mut content: Value =
        serde_json::from_str(candidate["content"].as_str().expect("ready content"))
            .expect("ready body");
    content["stream"] = Value::String("other-stream".to_owned());
    candidate["content"] = Value::String(serde_json::to_string(&content).expect("ready body"));
    let incompatible = block_on(port.sign(&candidate, None)).expect("sign incompatible rebind");
    port.stored.push(incompatible);

    let submissions = port.submissions.len();
    let error = block_on(assign_writer(
        &mut port,
        AssignWriterRequest {
            repo,
            issue,
            writer,
        },
    ))
    .expect_err("incompatible ready rebind must fail closed");
    assert_eq!(error.to_string(), "bw:reject:causality:ready-rebind");
    assert_eq!(port.submissions.len(), submissions);
}

fn assert_assignment_refused_in_state(labels: Vec<&'static str>) {
    let mut port = FakePort::from_history(labels, [1_800_000_010, 1_800_000_011, 1_800_000_012], 1);
    let (repo, issue) = coordinates(&port);
    let writer = port.fixtures["events"]["reassign"]["event"]["tags"][2][1]
        .as_str()
        .expect("new writer")
        .to_owned();

    let error = block_on(assign_writer(
        &mut port,
        AssignWriterRequest {
            repo,
            issue,
            writer,
        },
    ))
    .expect_err("assignment outside backlog or ready must fail closed");

    assert_eq!(error.to_string(), "bw:reject:causality:assignment-state");
    assert!(port.submissions.is_empty());
}

#[test]
fn assignment_in_triage_fails_closed_without_publishing() {
    assert_assignment_refused_in_state(vec!["repo", "policy", "root_a", "enroll_a"]);
}

#[test]
fn assignment_in_development_fails_closed_without_publishing() {
    assert_assignment_refused_in_state(vec![
        "repo",
        "policy",
        "root_a",
        "enroll_a",
        "update_a",
        "accept_a",
        "backlog_a",
        "assign_a",
        "ready_a",
        "dev_a",
    ]);
}

#[test]
fn assignment_in_implemented_fails_closed_without_publishing() {
    assert_assignment_refused_in_state(vec![
        "repo",
        "policy",
        "root_a",
        "enroll_a",
        "update_a",
        "accept_a",
        "backlog_a",
        "assign_a",
        "ready_a",
        "dev_a",
        "implemented_a",
    ]);
}
