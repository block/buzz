use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use buzz_command_sources_pkg::mcp_http::McpHttpClient;
use buzz_command_sources_pkg::usage::{
    UsageAdmission, UsageError, UsagePool, WorldMonitorUsageLedger,
};
use buzz_command_sources_pkg::world_monitor::{
    NormalizedWorldMonitorEvidence, WorldMonitorRequest, WorldMonitorTool,
};
use chrono::{DateTime, Local, SecondsFormat, Utc};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};
use tokio_util::sync::CancellationToken;

use super::canonical::CandidateSource;

const COLLECTION: &str = "world_monitor";
const MAX_FOCUS_COUNTRIES: usize = 5;

pub(super) struct WorldMonitorBriefBatch {
    pub(super) candidates: Vec<CandidateSource>,
    pub(super) limitations: Vec<String>,
    pub(super) quota_limited: bool,
}

trait WorldMonitorExecutor: Send + Sync {
    fn execute(&self, request: &WorldMonitorRequest) -> Result<Value, &'static str>;
}

struct HttpWorldMonitorExecutor {
    client: McpHttpClient,
}

impl WorldMonitorExecutor for HttpWorldMonitorExecutor {
    fn execute(&self, request: &WorldMonitorRequest) -> Result<Value, &'static str> {
        tauri::async_runtime::block_on(
            self.client
                .call_tool(request.tool.as_str(), request.arguments.clone()),
        )
        .map_err(|_| "world_monitor_unavailable")
    }
}

#[derive(Clone)]
pub(crate) struct WorldMonitorBriefCollector {
    executor: Option<Arc<dyn WorldMonitorExecutor>>,
    ledger_path: PathBuf,
}

impl WorldMonitorBriefCollector {
    pub(crate) fn from_app(app: &AppHandle, endpoint: &str) -> Self {
        let ledger_path = app
            .path()
            .app_config_dir()
            .map(|directory| directory.join("world-monitor-usage.json"))
            .unwrap_or_else(|_| std::env::temp_dir().join("world-monitor-usage-unavailable.json"));
        let executor =
            crate::secret_store::SecretStore::shared(crate::app_state::keyring_service())
                .load(buzz_command_sources_pkg::WORLD_MONITOR_KEYCHAIN_KEY)
                .ok()
                .flatten()
                .and_then(|api_key| McpHttpClient::world_monitor(endpoint, api_key).ok())
                .map(|client| {
                    Arc::new(HttpWorldMonitorExecutor { client }) as Arc<dyn WorldMonitorExecutor>
                });
        Self {
            executor,
            ledger_path,
        }
    }

    pub(super) fn unavailable(ledger_path: PathBuf) -> Self {
        Self {
            executor: None,
            ledger_path,
        }
    }

    pub(super) fn collect(
        &self,
        focus_text: &str,
        observed_at: &str,
        cancellation: &CancellationToken,
    ) -> WorldMonitorBriefBatch {
        let Some(executor) = &self.executor else {
            return WorldMonitorBriefBatch {
                candidates: Vec::new(),
                limitations: vec![
                    "World Monitor is not configured for the Maritime N2 update.".to_string(),
                ],
                quota_limited: false,
            };
        };
        let observed = DateTime::parse_from_rfc3339(observed_at)
            .map(|time| time.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());
        let ledger = WorldMonitorUsageLedger::new(self.ledger_path.clone());
        let now_local = Local::now();
        let mut candidates = Vec::new();
        let mut limitations = BTreeSet::new();
        let mut quota_limited = false;

        for request in daily_request_plan(focus_text) {
            if cancellation.is_cancelled() {
                break;
            }
            let evidence = match ledger.admit(UsagePool::Brief, &request, now_local) {
                Ok(UsageAdmission::Cached(evidence)) => evidence,
                Ok(UsageAdmission::Reserved { cache_key, .. }) => {
                    let payload = match executor.execute(&request) {
                        Ok(payload) => payload,
                        Err(code) => {
                            limitations.insert(format!(
                                "World Monitor {} was unavailable: {code}.",
                                request.tool.as_str()
                            ));
                            continue;
                        }
                    };
                    let evidence =
                        NormalizedWorldMonitorEvidence::new(request.clone(), payload, observed);
                    if ledger
                        .store_success(&cache_key, &evidence, now_local)
                        .is_err()
                    {
                        limitations.insert(
                            "World Monitor cache state could not be updated; retrieved evidence remains available for this brief."
                                .to_string(),
                        );
                    }
                    evidence
                }
                Err(UsageError::Exhausted) => {
                    quota_limited = true;
                    limitations.insert(
                        "World Monitor brief allowance is exhausted for today; remaining update calls were skipped."
                            .to_string(),
                    );
                    break;
                }
                Err(UsageError::State) => {
                    limitations.insert(
                        "World Monitor usage state was unavailable; no unmetered update calls were made."
                            .to_string(),
                    );
                    break;
                }
            };
            if let Some(candidate) = candidate_from_evidence(evidence, observed_at) {
                candidates.push(candidate);
            } else {
                limitations.insert(
                    "A malformed World Monitor result was excluded from the Intelligence evidence."
                        .to_string(),
                );
            }
        }

        WorldMonitorBriefBatch {
            candidates,
            limitations: limitations.into_iter().collect(),
            quota_limited,
        }
    }

    #[cfg(test)]
    fn for_test(executor: Arc<dyn WorldMonitorExecutor>, ledger_path: PathBuf) -> Self {
        Self {
            executor: Some(executor),
            ledger_path,
        }
    }
}

fn daily_request_plan(focus_text: &str) -> Vec<WorldMonitorRequest> {
    let mut requests = [
        (
            WorldMonitorTool::ConflictEvents,
            json!({"days": 7, "limit": 30}),
        ),
        (
            WorldMonitorTool::NewsIntelligence,
            json!({"topic": "conflict", "days": 7, "limit": 25}),
        ),
        (
            WorldMonitorTool::NewsIntelligence,
            json!({"topic": "economy", "days": 7, "limit": 25}),
        ),
        (
            WorldMonitorTool::NewsIntelligence,
            json!({"topic": "intelligence", "days": 7, "limit": 25}),
        ),
        (
            WorldMonitorTool::NewsIntelligence,
            json!({"topic": "maritime", "days": 7, "limit": 25}),
        ),
        (WorldMonitorTool::MilitaryPosture, json!({"limit": 25})),
        (WorldMonitorTool::ChokepointStatus, json!({"limit": 25})),
        (WorldMonitorTool::SupplyChainData, json!({"limit": 25})),
    ]
    .into_iter()
    .filter_map(|(tool, arguments)| WorldMonitorRequest::new(tool, arguments).ok())
    .collect::<Vec<_>>();
    for country_code in focus_country_codes(focus_text) {
        for (tool, arguments) in [
            (
                WorldMonitorTool::CountryRisk,
                json!({"country_code": country_code}),
            ),
            (
                WorldMonitorTool::MaritimeActivity,
                json!({"country_code": country_code, "limit": 25}),
            ),
            (
                WorldMonitorTool::NewsIntelligence,
                json!({"country_code": country_code, "days": 7, "limit": 25}),
            ),
        ] {
            if let Ok(request) = WorldMonitorRequest::new(tool, arguments) {
                requests.push(request);
            }
        }
    }
    requests
}

fn focus_country_codes(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut codes = BTreeSet::new();
    for window in bytes.windows(4) {
        if window[0] == b'('
            && window[3] == b')'
            && window[1].is_ascii_uppercase()
            && window[2].is_ascii_uppercase()
        {
            codes.insert(String::from_utf8_lossy(&window[1..3]).to_string());
        }
    }
    for marker in ["\"country_code\"", "\"countryCode\""] {
        let mut remainder = text;
        while let Some(index) = remainder.find(marker) {
            remainder = &remainder[index + marker.len()..];
            let Some(colon) = remainder.find(':') else {
                break;
            };
            let value = remainder[colon + 1..].trim_start();
            let value = value.strip_prefix('"').unwrap_or(value);
            let code = value.as_bytes().get(..2);
            if let Some(code) = code.filter(|code| code.iter().all(u8::is_ascii_uppercase)) {
                codes.insert(String::from_utf8_lossy(code).to_string());
            }
        }
    }
    codes.into_iter().take(MAX_FOCUS_COUNTRIES).collect()
}

fn candidate_from_evidence(
    evidence: NormalizedWorldMonitorEvidence,
    observed_at: &str,
) -> Option<CandidateSource> {
    let canonical_arguments = serde_jcs::to_vec(&evidence.arguments).ok()?;
    let canonical_payload = serde_jcs::to_vec(&evidence.payload).ok()?;
    let identity = hex::encode(Sha256::digest(
        [
            b"world_monitor:".as_slice(),
            evidence.tool.as_str().as_bytes(),
            b":",
            &canonical_arguments,
            b":",
            Sha256::digest(&canonical_payload).as_slice(),
        ]
        .concat(),
    ));
    let source_id = format!("world-monitor:{identity}");
    let retrieved_at = evidence
        .retrieved_at
        .to_rfc3339_opts(SecondsFormat::Millis, true);
    let timestamp = evidence
        .source_time
        .unwrap_or(evidence.retrieved_at)
        .to_rfc3339_opts(SecondsFormat::Millis, true);
    let quote = serde_jcs::to_vec(&json!({
        "provider": "World Monitor",
        "tool": evidence.tool,
        "arguments": evidence.arguments,
        "freshness": evidence.freshness,
        "payload": evidence.payload,
    }))
    .ok()
    .and_then(|bytes| String::from_utf8(bytes).ok())?;
    Some(CandidateSource {
        source_id: source_id.clone(),
        source_kind: super::SourceKind::WorldMonitor,
        collection: COLLECTION.to_string(),
        document_id: evidence.tool.as_str().to_string(),
        chunk_id: source_id,
        timestamp,
        location: format!(
            "World Monitor {} response; freshness={:?}",
            evidence.tool.as_str(),
            evidence.freshness
        )
        .to_lowercase(),
        retrieved_at,
        observed_at: observed_at.to_string(),
        quote,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct FakeExecutor {
        requests: Mutex<Vec<WorldMonitorRequest>>,
    }

    impl WorldMonitorExecutor for FakeExecutor {
        fn execute(&self, request: &WorldMonitorRequest) -> Result<Value, &'static str> {
            self.requests
                .lock()
                .expect("requests")
                .push(request.clone());
            Ok(json!({"items": [], "timestamp": "2026-07-28T00:00:00Z"}))
        }
    }

    #[test]
    fn deterministic_plan_is_eight_global_and_caps_five_countries_at_twenty_three() {
        assert_eq!(daily_request_plan("No focus country.").len(), 8);
        let plan =
            daily_request_plan(r#"Focus (PH) (JP) (AU) (NZ) (ID) (SG) and {"country_code":"PH"}."#);
        assert_eq!(plan.len(), 23);
        assert_eq!(
            plan.iter()
                .filter_map(|request| request.arguments["country_code"].as_str())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["AU", "ID", "JP", "NZ", "PH"])
        );
    }

    #[test]
    fn missing_key_degrades_without_network_or_quota_use() {
        let directory = tempfile::tempdir().expect("tempdir");
        let collector =
            WorldMonitorBriefCollector::unavailable(directory.path().join("usage.json"));
        let batch = collector.collect(
            "No focus.",
            "2026-07-28T06:00:00+10:00",
            &CancellationToken::new(),
        );
        assert!(batch.candidates.is_empty());
        assert!(batch.limitations[0].contains("not configured"));
        assert!(!directory.path().join("usage.json").exists());
    }

    #[test]
    fn collector_executes_plan_and_reuses_cache_without_spending_again() {
        let directory = tempfile::tempdir().expect("tempdir");
        let executor = Arc::new(FakeExecutor::default());
        let collector = WorldMonitorBriefCollector::for_test(
            executor.clone(),
            directory.path().join("usage.json"),
        );
        let first = collector.collect(
            "Deployment focus (PH).",
            "2026-07-28T06:00:00+10:00",
            &CancellationToken::new(),
        );
        assert_eq!(first.candidates.len(), 11);
        assert_eq!(executor.requests.lock().expect("requests").len(), 11);
        let second = collector.collect(
            "Deployment focus (PH).",
            "2026-07-28T06:05:00+10:00",
            &CancellationToken::new(),
        );
        assert_eq!(second.candidates.len(), 11);
        assert_eq!(executor.requests.lock().expect("requests").len(), 11);
        let snapshot = WorldMonitorUsageLedger::new(directory.path().join("usage.json"))
            .snapshot(Local::now())
            .expect("snapshot");
        assert_eq!(snapshot.brief_used, 11);
    }
}
