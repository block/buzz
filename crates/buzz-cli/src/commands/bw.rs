use crate::{client::BuzzClient, error::CliError};
use buzz_core::bw::{parse_json, Consumer, Evidence, Trust};
use nostr::{EventBuilder, JsonUtil, Kind, Tag, Timestamp};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

const LIVE_KINDS: [u16; 5] = [30617, 1621, 46100, 1063, 1];
const LIVE_HISTORY_LIMIT: u32 = 100_001;

#[derive(clap::Subcommand)]
pub(crate) enum BwCmd {
    /// Validate ordered signed events offline using explicit trust, evidence and now.
    Validate {
        #[arg(long)]
        input: PathBuf,
    },
    /// Display the recomputed projection of an offline history.
    Show {
        #[arg(long)]
        input: PathBuf,
    },
    /// Shape-check an unsigned draft; never sign or publish.
    DryRun {
        #[arg(long)]
        input: PathBuf,
    },
    /// Execute every named corpus case and export computed decisions (not expectations).
    Corpus {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Prepare and confirm a single activation-bound, readback-proven operation.
    ///
    /// Performs no network access. Without `--signed` it only pins the event ID
    /// the signature will carry; with `--signed` it also requires `--readback`,
    /// because publication is proven by an exact relay readback and nothing else.
    Publish {
        /// Bundle of trust, external evidence, now, signed history and operation.
        #[arg(long)]
        input: PathBuf,
        /// The signed event produced for the prepared operation.
        #[arg(long)]
        signed: Option<PathBuf>,
        /// The event read back from the relay under the pinned ID.
        #[arg(long, requires = "signed")]
        readback: Option<PathBuf>,
    },
}
fn input(path: &PathBuf) -> Result<Value, CliError> {
    let bytes = std::fs::read(path).map_err(|e| CliError::Usage(format!("read BW input: {e}")))?;
    parse_json(&bytes).map_err(|e| CliError::Usage(format!("strict BW JSON: {e}")))
}
fn consumer(v: &Value) -> Result<Consumer, CliError> {
    Ok(Consumer::new(
        serde_json::from_value(v["trust"].clone())
            .map_err(|e| CliError::Usage(format!("BW trust: {e}")))?,
        serde_json::from_value(v["external"].clone())
            .map_err(|e| CliError::Usage(format!("BW evidence: {e}")))?,
        v["now"]
            .as_u64()
            .filter(|n| *n <= u32::MAX as u64)
            .ok_or_else(|| CliError::Usage("explicit BW now required".into()))?,
    ))
}
fn wire(v: &Value) -> Result<Vec<u8>, CliError> {
    if let Some(raw) = v.as_str() {
        Ok(raw.as_bytes().to_vec())
    } else {
        serde_json::to_vec(v).map_err(|e| CliError::Usage(e.to_string()))
    }
}
fn evaluate(v: &Value) -> Result<(Value, Value), CliError> {
    let mut c = consumer(v)?;
    let events = v["events"]
        .as_array()
        .ok_or_else(|| CliError::Usage("BW events array required".into()))?;
    let mut results = Vec::new();
    for event in events {
        results.push(c.ingest(&wire(event)?));
    }
    Ok((json!(results), c.projection()))
}
fn corpus(v: &Value) -> Result<Value, CliError> {
    let cases = v["cases"]
        .as_array()
        .ok_or_else(|| CliError::Usage("BW cases required".into()))?;
    let mut results = Vec::new();
    for case in cases {
        let mut c = consumer(case)?;
        let mut steps = Vec::new();
        for label in case["input"]
            .as_array()
            .ok_or_else(|| CliError::Usage("case input required".into()))?
        {
            let label = label
                .as_str()
                .ok_or_else(|| CliError::Usage("event label required".into()))?;
            let event = v["events"][label]
                .get("event")
                .ok_or_else(|| CliError::Usage(format!("missing event {label}")))?;
            steps.push(c.ingest(&wire(event)?));
        }
        results.push(json!({"case":case["name"],"steps":steps}));
    }
    Ok(json!({"format":"nip-bw-results-v1","cases":results}))
}
/// Surface a BW producer refusal as its stable `bw:` code alone.
///
/// The vocabulary is closed and carries no event content, key material or
/// signature bytes, so a caller can branch on it and a log can retain it.
fn refusal(e: buzz_sdk::SdkError) -> CliError {
    CliError::Usage(match e {
        buzz_sdk::SdkError::InvalidInput(code) => code,
        other => other.to_string(),
    })
}
/// Load a single signed event from disk into the strict BW wire form.
fn event(path: &PathBuf) -> Result<nostr::Event, CliError> {
    let v = input(path)?;
    nostr::Event::from_json(v.to_string()).map_err(|_| CliError::Usage("bw:envelope:fields".into()))
}
/// Rebuild the offline consumer from an explicit bundle: externally confirmed
/// trust, external observations, explicit now and the signed history. Individual
/// history outcomes are not asserted here — activation revalidates what it needs.
fn history(v: &Value) -> Result<Consumer, CliError> {
    let mut c = consumer(v)?;
    for e in v["events"].as_array().unwrap_or(&Vec::new()) {
        c.ingest(&wire(e)?);
    }
    Ok(c)
}

fn tag<'a>(event: &'a Value, name: &str) -> Option<&'a str> {
    event["tags"]
        .as_array()?
        .iter()
        .find(|tag| tag[0] == name)?[1]
        .as_str()
}

fn empty_evidence() -> Evidence {
    Evidence {
        git_readbacks: vec![],
        git_ancestry: vec![],
        provider_readbacks: vec![],
        downloads: vec![],
        host_authorization: json!({"allowed": false}),
    }
}

fn now() -> Result<u64, CliError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| CliError::Other(format!("read system clock: {error}")))
}

fn repo_coordinate(repo_owner: &str, repo_id: &str) -> Result<String, CliError> {
    crate::validate::validate_hex64(repo_owner)?;
    crate::validate::validate_repo_id(repo_id)?;
    Ok(format!(
        "30617:{}:{repo_id}",
        repo_owner.to_ascii_lowercase()
    ))
}

fn collect_reference_ids(value: &Value, ids: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                if matches!(
                    key.as_str(),
                    "assignment"
                        | "update"
                        | "implemented"
                        | "pipeline"
                        | "request"
                        | "run"
                        | "artifact"
                        | "set"
                        | "target"
                        | "issue"
                ) {
                    if let Some(id) = value.as_str() {
                        ids.insert(id.to_owned());
                    }
                }
                collect_reference_ids(value, ids);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_reference_ids(value, ids);
            }
        }
        _ => {}
    }
}

fn event_id(event: &Value) -> Result<String, CliError> {
    event["id"]
        .as_str()
        .filter(|id| id.len() == 64 && id.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| CliError::Other("BW history event has no valid id".into()))
}

/// Load a complete repository-scoped BW history through the generic Nostr
/// query bridge. Candidate kinds are enumerated before local `a`-tag
/// filtering, so a relay that mishandles tag pagination cannot silently make
/// an incomplete history look authoritative. Exact referenced events are then
/// closed over recursively, including foreign bindings that Core must reject.
async fn live_history(client: &BuzzClient, repo: &str) -> Result<(Consumer, Evidence), CliError> {
    let mut parts = repo.splitn(3, ':');
    let kind = parts.next();
    let owner = parts.next();
    let repo_id = parts.next();
    let (Some("30617"), Some(owner), Some(repo_id)) = (kind, owner, repo_id) else {
        return Err(CliError::Usage("invalid BW repository coordinate".into()));
    };
    crate::validate::validate_hex64(owner)?;
    crate::validate::validate_repo_id(repo_id)?;

    let mut events = BTreeMap::<String, Value>::new();
    for kind in LIVE_KINDS {
        let candidates = client
            .query_paginated(json!({"kinds": [kind]}), LIVE_HISTORY_LIMIT)
            .await?;
        if candidates.len() >= LIVE_HISTORY_LIMIT as usize {
            return Err(CliError::Other(
                "BW history incomplete: candidate scan limit".into(),
            ));
        }
        for event in candidates {
            let belongs = if kind == 30617 {
                event["pubkey"]
                    .as_str()
                    .is_some_and(|pubkey| pubkey.eq_ignore_ascii_case(owner))
                    && tag(&event, "d") == Some(repo_id)
            } else {
                tag(&event, "a") == Some(repo)
            };
            if belongs {
                events.insert(event_id(&event)?, event);
            }
        }
    }

    let mut requested = BTreeSet::new();
    loop {
        let mut references = BTreeSet::new();
        for event in events.values() {
            for tag in event["tags"].as_array().into_iter().flatten() {
                if matches!(
                    tag[0].as_str(),
                    Some(
                        "issue"
                            | "policy"
                            | "previous"
                            | "prior"
                            | "delegation"
                            | "set"
                            | "run"
                            | "e"
                    )
                ) {
                    if let Some(id) = tag[1].as_str() {
                        references.insert(id.to_owned());
                    }
                }
            }
            if event["kind"] == 46100 {
                if let Some(content) = event["content"].as_str() {
                    if let Ok(body) = parse_json(content.as_bytes()) {
                        collect_reference_ids(&body, &mut references);
                    }
                }
            }
        }
        let references = references
            .into_iter()
            .filter(|id| {
                id.len() == 64
                    && id.bytes().all(|byte| byte.is_ascii_hexdigit())
                    && requested.insert(id.to_ascii_lowercase())
            })
            .collect::<Vec<_>>();
        if references.is_empty() {
            break;
        }
        for ids in references.chunks(100) {
            for event in client
                .query_paginated(json!({"kinds": LIVE_KINDS, "ids": ids}), LIVE_HISTORY_LIMIT)
                .await?
            {
                events.insert(event_id(&event)?, event);
            }
        }
    }

    let evidence = empty_evidence();
    let mut consumer = Consumer::new(
        Trust {
            community: client.relay_url().to_owned(),
            repo: repo.to_owned(),
            owner: owner.to_ascii_lowercase(),
        },
        evidence.clone(),
        now()?,
    );
    for event in events.into_values() {
        let bytes = serde_json::to_vec(&event)
            .map_err(|error| CliError::Other(format!("serialize BW history: {error}")))?;
        consumer.ingest(&bytes);
    }
    Ok((consumer, evidence))
}

fn prepared_start_development(
    consumer: &Consumer,
    repo: &str,
    issue: &str,
    signer: &str,
    created_at: u64,
    auth_tag: Option<&Tag>,
) -> Result<(buzz_sdk::bw::Publication, Value), CliError> {
    let projection = consumer.projection();
    let previous = projection["issue_state_id"][issue]
        .as_str()
        .ok_or_else(|| CliError::Usage("issue has no current BW state".into()))?;
    let state = &projection["issue_state"][issue];
    if state["state"] != "ready" {
        return Err(CliError::Usage("BW issue is not ready".into()));
    }
    let stream = state["stream"]
        .as_str()
        .ok_or_else(|| CliError::Usage("ready issue has no stream".into()))?;
    let assignment = state["assignment"]
        .as_str()
        .ok_or_else(|| CliError::Usage("ready issue has no selected writer".into()))?;
    let activation = consumer.activation().map_err(|error| {
        CliError::Usage(format!(
            "bw:{}:{}:{}",
            error.outcome, error.stage, error.code
        ))
    })?;
    let mut tags = vec![
        vec!["a".to_owned(), repo.to_owned()],
        vec!["policy".to_owned(), activation.policy],
        vec!["issue".to_owned(), issue.to_owned()],
        vec!["previous".to_owned(), previous.to_owned()],
    ];
    if let Some(auth_tag) = auth_tag {
        tags.push(auth_tag.as_slice().to_vec());
    }
    let draft = buzz_sdk::bw::record(
        buzz_sdk::bw::RecordType::IssueState,
        tags,
        json!({
            "state": "in-development",
            "stream": stream,
            "assignment": assignment,
        }),
    )
    .map_err(refusal)?;
    let dry = draft.dry_run().map_err(refusal)?;
    let candidate = json!({
        "pubkey": signer,
        "created_at": created_at,
        "kind": dry["kind"],
        "tags": dry["tags"],
        "content": dry["content"],
    });
    let publication = buzz_sdk::bw::Publication::prepare(consumer, candidate).map_err(refusal)?;
    Ok((publication, dry))
}

fn parsed_tags(dry: &Value) -> Result<Vec<Tag>, CliError> {
    dry["tags"]
        .as_array()
        .ok_or_else(|| CliError::Other("bw:shape:tags".into()))?
        .iter()
        .map(|tag| {
            let parts = tag
                .as_array()
                .ok_or_else(|| CliError::Other("bw:shape:tags".into()))?
                .iter()
                .map(|part| {
                    part.as_str()
                        .ok_or_else(|| CliError::Other("bw:shape:tags".into()))
                })
                .collect::<Result<Vec<_>, _>>()?;
            Tag::parse(parts).map_err(|error| CliError::Other(format!("invalid BW tag: {error}")))
        })
        .collect()
}

/// Let the selected writer identity perform the live `ready` →
/// `in-development` transition itself. This is the agent-facing counterpart
/// to Desktop's asynchronous request button; no owner impersonation or local
/// managed-agent key lookup is involved.
pub(crate) async fn start_development(
    client: &BuzzClient,
    issue: &str,
    repo_owner: &str,
    repo_id: &str,
) -> Result<(), CliError> {
    crate::validate::validate_hex64(issue)?;
    let issue = issue.to_ascii_lowercase();
    let repo = repo_coordinate(repo_owner, repo_id)?;
    let (mut consumer, evidence) = live_history(client, &repo).await?;
    let created_at = now()?;
    consumer.observe(evidence, created_at);
    let signer = client.keys().public_key().to_hex();
    let (publication, dry) = prepared_start_development(
        &consumer,
        &repo,
        &issue,
        &signer,
        created_at,
        client.bw_auth_tag(),
    )?;
    let content = dry["content"]
        .as_str()
        .ok_or_else(|| CliError::Other("bw:shape:content".into()))?;
    let event = EventBuilder::new(Kind::Custom(46100), content)
        .tags(parsed_tags(&dry)?)
        .custom_created_at(Timestamp::from_secs(created_at))
        .sign_with_keys(client.keys())
        .map_err(|error| CliError::Other(format!("sign BW record: {error}")))?;
    publication.seal(&event).map_err(refusal)?;

    let submit_error = client.submit_event(event.clone()).await.err();
    let readback = client
        .query_paginated(
            json!({"ids": [publication.event_id()], "kinds": [46100]}),
            2,
        )
        .await?
        .into_iter()
        .find(|candidate| candidate["id"].as_str() == Some(publication.event_id()))
        .map(serde_json::from_value::<nostr::Event>)
        .transpose()
        .map_err(|error| CliError::Other(format!("invalid BW readback: {error}")))?;
    if readback.is_none() {
        if let Some(error) = submit_error {
            return Err(error);
        }
    }
    publication
        .confirm(&mut consumer, &event, readback.as_ref())
        .map_err(refusal)?;
    println!(
        "{}",
        json!({
            "event_id": publication.event_id(),
            "accepted": true,
            "message": "issue moved to in-development",
            "state": "in-development",
        })
    );
    Ok(())
}
/// The readback-bound publish boundary: prepare, seal, confirm. No I/O, no
/// signing and no local success latch — the relay readback is the only proof.
fn publish(
    path: &PathBuf,
    signed: Option<&PathBuf>,
    readback: Option<&PathBuf>,
) -> Result<Value, CliError> {
    let v = input(path)?;
    let mut consumer = history(&v)?;
    let publication =
        buzz_sdk::bw::Publication::prepare(&consumer, v["operation"].clone()).map_err(refusal)?;
    let activation = serde_json::to_value(publication.activation())
        .map_err(|e| CliError::Other(e.to_string()))?;
    let Some(signed) = signed else {
        return Ok(
            json!({"stage":"prepared","event_id":publication.event_id(),"activation":activation,"published":false}),
        );
    };
    let signed = event(signed)?;
    let readback = readback
        .ok_or_else(|| CliError::Usage("bw:readback:missing".into()))
        .and_then(event)?;
    publication
        .confirm(&mut consumer, &signed, Some(&readback))
        .map_err(refusal)?;
    Ok(
        json!({"stage":"published","event_id":publication.event_id(),"activation":activation,"published":true}),
    )
}
pub(crate) fn dispatch(cmd: &BwCmd) -> Result<(), CliError> {
    let output = match cmd {
        BwCmd::Publish {
            input: path,
            signed,
            readback,
        } => publish(path, signed.as_ref(), readback.as_ref())?,
        BwCmd::DryRun { input: path } => {
            let v = input(path)?;
            let draft = buzz_sdk::bw::Draft::new(
                v["kind"]
                    .as_u64()
                    .ok_or_else(|| CliError::Usage("draft kind required".into()))?,
                v["tags"].clone(),
                v["content"]
                    .as_str()
                    .ok_or_else(|| CliError::Usage("draft content required".into()))?
                    .into(),
            )
            .map_err(|e| CliError::Usage(e.to_string()))?;
            draft
                .dry_run()
                .map_err(|e| CliError::Usage(e.to_string()))?
        }
        BwCmd::Validate { input: path } => evaluate(&input(path)?)?.0,
        BwCmd::Show { input: path } => evaluate(&input(path)?)?.1,
        BwCmd::Corpus {
            input: path,
            output,
        } => {
            let bytes = std::fs::read(path).map_err(|e| CliError::Usage(e.to_string()))?;
            let digest = hex::encode(Sha256::digest(&bytes));
            if digest != "b489fb188b45e073e81783e8d591f24891a9c73bc56d8d2fc56a881606640e8a" {
                return Err(CliError::Usage(
                    "corpus differs from pinned P1; contract review required".into(),
                ));
            }
            let data = parse_json(&bytes).map_err(|e| CliError::Usage(e.to_string()))?;
            let mut result = corpus(&data)?;
            result["p1_commit"] = json!("529136c39aa42db64af61f811fc33dac73d16dd1");
            result["fixture_sha256"] = json!(digest);
            result["document_sha256"] =
                json!("86184a747dda4692c64db65c216f4cdb6dfe69e765755abd1eabba893df55d29");
            let bytes =
                serde_json::to_vec_pretty(&result).map_err(|e| CliError::Other(e.to_string()))?;
            std::fs::write(output, bytes)
                .map_err(|e| CliError::Other(format!("BW export: {e}")))?;
            json!({"output":output,"cases":result["cases"].as_array().map(Vec::len),"publish_enabled":false})
        }
    };
    println!("{output}");
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn offline_json_is_independent_of_oracles() {
        let mut v: Value =
            serde_json::from_str(include_str!("../../../../docs/nips/NIP-BW.fixtures.json"))
                .expect("fixture");
        v["cases"] = json!([v["cases"][1]]);
        let first = corpus(&v).expect("execute");
        v["cases"][0]["expected"] = json!([{ "outcome":"invented" }]);
        assert_eq!(corpus(&v).expect("execute"), first);
        assert_eq!(
            first["cases"][0]["steps"][0]["event_id"],
            v["events"]["repo"]["event"]["id"]
        );
    }
    fn fixtures() -> Value {
        serde_json::from_str(include_str!("../../../../docs/nips/NIP-BW.fixtures.json"))
            .expect("fixture")
    }
    /// Write a publish bundle for a pinned corpus case plus one unsigned operation.
    fn bundle(dir: &std::path::Path, case: &str, labels: &[&str], operation: &str) -> PathBuf {
        let f = fixtures();
        let c = f["cases"]
            .as_array()
            .expect("cases")
            .iter()
            .find(|c| c["name"] == case)
            .expect("case");
        let e = &f["events"][operation]["event"];
        let bundle = json!({
            "trust": c["trust"],
            "external": c["external"],
            "now": c["now"],
            "events": labels.iter().map(|l| f["events"][l]["event"].clone()).collect::<Vec<_>>(),
            "operation": {"pubkey":e["pubkey"],"created_at":e["created_at"],"kind":e["kind"],"tags":e["tags"],"content":e["content"]},
        });
        write(dir, "bundle.json", &bundle)
    }
    fn write(dir: &std::path::Path, name: &str, v: &Value) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, serde_json::to_vec(v).expect("json")).expect("write");
        path
    }
    fn arg(p: &std::path::Path) -> String {
        p.to_str().expect("path").to_owned()
    }

    fn fixture_consumer_through(f: &Value, last_label: &str) -> Consumer {
        let case = f["cases"]
            .as_array()
            .expect("cases")
            .iter()
            .find(|case| case["name"] == "issue-state-positive")
            .expect("issue-state-positive");
        let mut consumer = Consumer::new(
            serde_json::from_value(case["trust"].clone()).expect("trust"),
            serde_json::from_value(case["external"].clone()).expect("evidence"),
            case["now"].as_u64().expect("now"),
        );
        for label in case["input"].as_array().expect("input") {
            let label = label.as_str().expect("label");
            consumer
                .ingest(&serde_json::to_vec(&f["events"][label]["event"]).expect("signed event"));
            if label == last_label {
                break;
            }
        }
        consumer
    }

    #[test]
    fn live_start_development_prepares_the_normative_writer_event() {
        let f = fixtures();
        let consumer = fixture_consumer_through(&f, "ready_a");
        let expected = &f["events"]["dev_a"]["event"];
        let (publication, dry) = prepared_start_development(
            &consumer,
            expected["tags"][1][1].as_str().expect("repo"),
            expected["tags"][3][1].as_str().expect("issue"),
            expected["pubkey"].as_str().expect("writer"),
            expected["created_at"].as_u64().expect("time"),
            None,
        )
        .expect("selected writer may start development");
        assert_eq!(publication.activation().policy, expected["tags"][2][1]);
        assert_eq!(dry["tags"], expected["tags"]);
        assert_eq!(
            parse_json(dry["content"].as_str().expect("content").as_bytes()).expect("body"),
            parse_json(expected["content"].as_str().expect("content").as_bytes()).expect("body"),
        );
    }

    #[test]
    fn live_start_development_refuses_a_non_writer_identity() {
        let f = fixtures();
        let consumer = fixture_consumer_through(&f, "ready_a");
        let expected = &f["events"]["dev_a"]["event"];
        let error = prepared_start_development(
            &consumer,
            expected["tags"][1][1].as_str().expect("repo"),
            expected["tags"][3][1].as_str().expect("issue"),
            f["events"]["repo"]["event"]["pubkey"]
                .as_str()
                .expect("owner"),
            expected["created_at"].as_u64().expect("time"),
            None,
        )
        .expect_err("owner cannot impersonate the writer");
        assert!(error.to_string().contains("bw:reject:role:unauthorized"));
    }

    #[tokio::test]
    async fn publish_needs_an_explicit_bundle() {
        // No bundle is no activation evidence; the historical gate stays closed.
        assert_eq!(crate::run_from_args(["buzz", "bw", "publish"]).await, 1);
    }
    #[tokio::test]
    async fn publish_prepares_pins_and_requires_an_exact_readback() {
        let dir = tempfile::tempdir().expect("tempdir");
        let f = fixtures();
        let input = bundle(
            dir.path(),
            "issue-update-positive",
            &["repo", "policy", "root_a", "enroll_a"],
            "update_a",
        );
        let event = write(dir.path(), "signed.json", &f["events"]["update_a"]["event"]);
        let foreign = write(
            dir.path(),
            "foreign.json",
            &f["events"]["enroll_a"]["event"],
        );
        // Preparation alone never publishes and never needs live access.
        assert_eq!(
            crate::run_from_args(["buzz", "bw", "publish", "--input", &arg(&input)]).await,
            0
        );
        // A signed event without a readback is not a success.
        assert_eq!(
            crate::run_from_args([
                "buzz",
                "bw",
                "publish",
                "--input",
                &arg(&input),
                "--signed",
                &arg(&event),
            ])
            .await,
            1
        );
        // A readback of a different event is not this event's readback.
        assert_eq!(
            crate::run_from_args([
                "buzz",
                "bw",
                "publish",
                "--input",
                &arg(&input),
                "--signed",
                &arg(&event),
                "--readback",
                &arg(&foreign),
            ])
            .await,
            1
        );
        // The exact readback of the pinned event is the only accepted proof.
        assert_eq!(
            crate::run_from_args([
                "buzz",
                "bw",
                "publish",
                "--input",
                &arg(&input),
                "--signed",
                &arg(&event),
                "--readback",
                &arg(&event),
            ])
            .await,
            0
        );
    }
    #[tokio::test]
    async fn publish_refuses_unauthorized_and_unactivated_bundles() {
        let dir = tempfile::tempdir().expect("tempdir");
        // No owner-signed genesis in the supplied history.
        let bare = bundle(dir.path(), "issue-update-positive", &["policy"], "update_a");
        assert_eq!(
            crate::run_from_args(["buzz", "bw", "publish", "--input", &arg(&bare)]).await,
            1
        );
        // Activated history, but the operation's signer holds no such role.
        let wrong = bundle(
            dir.path(),
            "issue-update-wrong-role",
            &["repo", "policy", "root_a", "enroll_a"],
            "issue-update-wrong-role",
        );
        assert_eq!(
            crate::run_from_args(["buzz", "bw", "publish", "--input", &arg(&wrong)]).await,
            1
        );
    }
    #[tokio::test]
    async fn offline_subcommands_stay_usable_without_relay_or_key() {
        let dir = tempfile::tempdir().expect("tempdir");
        let f = fixtures();
        let c = f["cases"].as_array().expect("cases")[2].clone();
        let history = write(
            dir.path(),
            "history.json",
            &json!({
                "trust": c["trust"],
                "external": c["external"],
                "now": c["now"],
                "events": c["input"].as_array().expect("input")
                    .iter().map(|l| f["events"][l.as_str().expect("label")]["event"].clone())
                    .collect::<Vec<_>>(),
            }),
        );
        // No BUZZ_RELAY_URL and no BUZZ_PRIVATE_KEY are consulted on these paths.
        for cmd in ["validate", "show"] {
            assert_eq!(
                crate::run_from_args(["buzz", "bw", cmd, "--input", &arg(&history)]).await,
                0,
                "{cmd}"
            );
        }
    }
}
