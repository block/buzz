//! Archives and requests deletion of the communities `seed` provisioned.

use std::collections::BTreeMap;

use futures_util::stream::{self, StreamExt};
use nostr::Keys;
use reqwest::StatusCode;
use serde_json::json;

use super::identity::{self, Role};
use super::net::{http_reason, Net};
use super::scenario::Scenario;
use super::seed;

/// The acknowledgement version the relay accepts for owner deletions.
const ACKNOWLEDGEMENT_VERSION: i32 = 1;

/// What `teardown` did across the scenario's communities.
#[derive(Debug, Default)]
pub struct Report {
    /// Deletion requests by the stage the relay reported.
    pub requested: BTreeMap<String, usize>,
    /// Communities with neither this run's fixture owner nor this run's
    /// deletion request: never seeded, or another owner's host.
    pub absent: usize,
    /// One reason per community whose archive or deletion request failed.
    pub failures: Vec<String>,
}

impl Report {
    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "requested": self.requested,
            "absent": self.absent,
            "failed": self.failures.len(),
            "failures": self.failures.iter().take(10).collect::<Vec<_>>(),
        })
    }
}

enum Outcome {
    Requested(String),
    Absent,
}

/// Archives every templated community of the scenario as its fixture owner,
/// then requests its deletion under a request id derived from `run_id` and
/// the host, so a re-run replays the same requests. Deletion itself runs
/// asynchronously on the relay's deletion drain.
pub async fn teardown(scenario: &Scenario, net: &Net) -> anyhow::Result<Report> {
    if scenario.target.community_host_template.is_none() {
        anyhow::bail!(
            "teardown deletes only communities from target.community_host_template; \
             this scenario targets {} directly",
            scenario.target.http_url
        );
    }
    let (base, operator) = seed::operator(scenario)?;
    Ok(teardown_with(scenario, net, base, &operator).await)
}

async fn teardown_with(scenario: &Scenario, net: &Net, base: &str, operator: &Keys) -> Report {
    let outcomes: Vec<Result<Outcome, String>> = stream::iter(scenario.communities())
        .filter_map(|community| async move { Some((community.index, community.host?)) })
        .map(|(index, host)| async move {
            teardown_community(net, base, operator, &scenario.run_id, index, &host).await
        })
        .buffer_unordered(scenario.shape.seed_concurrency)
        .collect()
        .await;
    let mut report = Report::default();
    for outcome in outcomes {
        match outcome {
            Ok(Outcome::Requested(stage)) => *report.requested.entry(stage).or_default() += 1,
            Ok(Outcome::Absent) => report.absent += 1,
            Err(reason) => report.failures.push(reason),
        }
    }
    report
}

async fn teardown_community(
    net: &Net,
    base: &str,
    operator: &Keys,
    run_id: &str,
    index: usize,
    host: &str,
) -> Result<Outcome, String> {
    let owner = identity::keys(run_id, Role::Owner, index)
        .public_key()
        .to_hex();
    let post = |path: &'static str, body: serde_json::Value| async move {
        let body = serde_json::to_vec(&body).map_err(|e| e.to_string())?;
        net.post(operator, base, path, body, None)
            .await
            .map_err(|e| format!("{path} {host}: {e}"))
    };

    let (status, text) = post(
        "/operator/communities/archive",
        json!({ "host": host, "owner_pubkey": owner }),
    )
    .await?;
    // Not found means this owner holds no live community here. A deletion
    // already past membership purge looks the same, so replay it below.
    if !status.is_success() && status != StatusCode::NOT_FOUND {
        return Err(format!("archive {host}: {}", http_reason(status, &text)));
    }

    let (status, text) = post(
        "/operator/communities/delete",
        json!({
            "host": host,
            "owner_pubkey": owner,
            "request_id": identity::teardown_request_id(run_id, host).to_string(),
            "acknowledgement_version": ACKNOWLEDGEMENT_VERSION,
        }),
    )
    .await?;
    if status == StatusCode::NOT_FOUND {
        return Ok(Outcome::Absent);
    }
    if !status.is_success() {
        return Err(format!("delete {host}: {}", http_reason(status, &text)));
    }
    let accepted: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("delete {host}: unreadable response: {e}"))?;
    let stage = accepted["status"].as_str().unwrap_or("unknown").to_string();
    Ok(Outcome::Requested(stage))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loadgen::seed::tests::fake_http;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Default)]
    struct Community {
        /// Empty once deletion has purged membership.
        owner: String,
        archived: bool,
        /// Request id and stage of the community's deletion request.
        deletion: Option<(String, &'static str)>,
    }

    /// The operator's archive and delete endpoints: owner-checked, archive
    /// before delete, and a request id replayed for its community at any
    /// stage, even after membership purge. A request id already used for
    /// another host, or a second id for one community, conflicts.
    /// `unavailable` answers every archive with 503.
    async fn fake_operator(
        communities: Arc<Mutex<HashMap<String, Community>>>,
        unavailable: &'static str,
    ) -> String {
        fake_http(move |_, target, body| {
            let request: serde_json::Value = serde_json::from_slice(body).unwrap();
            let host = request["host"].as_str().unwrap();
            let owner = request["owner_pubkey"].as_str().unwrap();
            let mut communities = communities.lock().unwrap();
            let not_found = (404, json!({"error": "community not found"}).to_string());
            match target {
                "/operator/communities/archive" => {
                    let Some(community) = communities.get_mut(host).filter(|c| c.owner == owner)
                    else {
                        return not_found;
                    };
                    if host == unavailable {
                        return (503, json!({"error": "propagation pending"}).to_string());
                    }
                    community.archived = true;
                    (200, json!({"status": "archived"}).to_string())
                }
                "/operator/communities/delete" => {
                    if request["acknowledgement_version"] != 1 {
                        return (400, json!({"error": "unsupported"}).to_string());
                    }
                    let id = request["request_id"].as_str().unwrap();
                    let known = communities.iter().find_map(|(h, c)| {
                        c.deletion
                            .as_ref()
                            .filter(|(known, _)| known == id)
                            .map(|(_, stage)| (h.as_str(), *stage))
                    });
                    match known {
                        Some((h, stage)) if h == host => {
                            return (202, json!({ "status": stage }).to_string());
                        }
                        Some(_) => {
                            return (
                                409,
                                json!({"code": "deletion_request_conflict"}).to_string(),
                            );
                        }
                        None => {}
                    }
                    let Some(community) = communities.get_mut(host).filter(|c| c.owner == owner)
                    else {
                        return not_found;
                    };
                    if !community.archived {
                        return (409, json!({"code": "community_not_archived"}).to_string());
                    }
                    if community.deletion.is_some() {
                        return (
                            409,
                            json!({"code": "deletion_request_conflict"}).to_string(),
                        );
                    }
                    community.deletion = Some((id.to_string(), "submitted"));
                    (202, json!({"status": "submitted"}).to_string())
                }
                other => panic!("unexpected {other}"),
            }
        })
        .await
    }

    fn scenario(addr: &str, template: Option<&str>) -> Scenario {
        let template = template
            .map(|t| format!("community_host_template = \"{t}\"\noperator_url = \"http://{addr}\""))
            .unwrap_or_default();
        Scenario::parse(&format!(
            r#"
run_id = "t"
[target]
ws_url = "ws://{addr}"
http_url = "http://{addr}"
{template}
[shape]
communities = {}
channels_per_community = 1
[population]
humans = 1
agents = 0
channels_per_member = 1
[[ramp]]
users = 1.0
hold_secs = 1
"#,
            if template.is_empty() { 1 } else { 5 }
        ))
        .unwrap()
    }

    #[tokio::test]
    async fn teardown_deletes_only_the_fixture_owners_communities_and_replays_on_rerun() {
        let owner = |i| identity::keys("t", Role::Owner, i).public_key().to_hex();
        let communities = Arc::new(Mutex::new(HashMap::from([
            (
                "lg-t-0.example".to_string(),
                Community {
                    owner: owner(0),
                    ..Default::default()
                },
            ),
            (
                // A host this run does not own is left alone.
                "lg-t-1.example".to_string(),
                Community {
                    owner: Keys::generate().public_key().to_hex(),
                    ..Default::default()
                },
            ),
            // lg-t-2 was never seeded.
            (
                "lg-t-3.example".to_string(),
                Community {
                    owner: owner(3),
                    ..Default::default()
                },
            ),
            (
                // Deletion already purged membership, so archive finds no
                // owner; the request is still replayed for its stage.
                "lg-t-4.example".to_string(),
                Community {
                    archived: true,
                    deletion: Some((
                        identity::teardown_request_id("t", "lg-t-4.example").to_string(),
                        "postgres_purged",
                    )),
                    ..Default::default()
                },
            ),
        ])));
        let addr = fake_operator(communities.clone(), "lg-t-3.example").await;
        let scenario = scenario(&addr, Some("lg-{run}-{i}.example"));
        let net = Net::new(&scenario).await.unwrap();
        let base = format!("http://{addr}");
        let operator = Keys::generate();

        for pass in ["first", "re-run"] {
            let report = teardown_with(&scenario, &net, &base, &operator).await;
            assert_eq!(
                report.requested,
                BTreeMap::from([("postgres_purged".into(), 1), ("submitted".into(), 1)]),
                "{pass}"
            );
            assert_eq!(report.absent, 2, "{pass}");
            assert_eq!(report.failures.len(), 1, "{pass}: {:?}", report.failures);
            assert!(
                report.failures[0].starts_with("archive lg-t-3.example: HTTP 503"),
                "{pass}: {:?}",
                report.failures
            );
        }
        // The same run under another host template names another owner's
        // host; it gets its own request id and is left alone.
        communities.lock().unwrap().insert(
            "other-0.example".to_string(),
            Community {
                owner: Keys::generate().public_key().to_hex(),
                ..Default::default()
            },
        );
        let other = scenario(&addr, Some("other-{i}.example"));
        let report = teardown_with(&other, &net, &base, &operator).await;
        assert!(report.failures.is_empty(), "{:?}", report.failures);
        assert_eq!(report.absent, 5);

        let communities = communities.lock().unwrap();
        assert!(!communities["other-0.example"].archived);
        assert_eq!(
            communities["lg-t-0.example"].deletion,
            Some((
                identity::teardown_request_id("t", "lg-t-0.example").to_string(),
                "submitted"
            ))
        );
        assert!(!communities["lg-t-1.example"].archived);
        assert!(communities["lg-t-1.example"].deletion.is_none());
        assert!(communities["lg-t-3.example"].deletion.is_none());
    }

    #[tokio::test]
    async fn teardown_refuses_a_scenario_without_templated_communities() {
        let scenario = scenario("127.0.0.1:9", None);
        let net = Net::new(&scenario).await.unwrap();
        let err = teardown(&scenario, &net).await.unwrap_err();
        assert!(err.to_string().contains("community_host_template"), "{err}");
    }
}
