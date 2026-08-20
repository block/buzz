use serde::{Deserialize, Serialize};

/// This node's own routing activity, plus the peers it can currently see.
///
/// Read-only projection of the node's own status payload — no new trust
/// surface.
///
/// **Direction matters.** Every counter here comes from `routing_metrics`,
/// which is incremented only by this node's local OpenAI ingress
/// (`network/openai/transport.rs`) when *this* node dispatches a request. The
/// inbound peer-serving path (`mesh/stage_transport.rs`) never touches it — it
/// only observes inflight. So the attempt/served split says *where my requests
/// ran*, never *who asked me*:
///
/// - local  → my own GPU ran it
/// - remote → a peer ran it for me (I am CONSUMING someone else's compute)
///
/// mesh-llm exposes no inbound counter (`fronted_request_count` merely aliases
/// `request_count`), so nothing here may be labelled "served for others".
/// Inbound work can only be *inferred*: serving, with inflight > 0, while our
/// own dispatch count stays flat, means the work is necessarily not ours.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct MeshServingUsage {
    /// Inference in flight on this node right now. Does NOT distinguish a local
    /// agent's request from a remote member's — the only live signal available.
    pub inflight: u64,
    /// Highest concurrent in-flight seen this session.
    pub peak_inflight: u64,
    /// Requests this node's ingress has routed (outbound), NOT work served for
    /// others. Named `requests_served` for wire compatibility; treat as
    /// "requests routed".
    pub requests_served: u64,
    /// Completion tokens this node observed on requests it dispatched.
    pub tokens_served: u64,
    /// Recent decode throughput.
    pub tokens_per_second: f64,
    /// Attempts this node dispatched to its own GPU.
    pub local_attempts: u64,
    /// Attempts this node dispatched to a peer — i.e. this machine consuming
    /// someone else's compute.
    pub remote_attempts: u64,
    /// Attempts this node dispatched to a configured endpoint.
    pub endpoint_attempts: u64,
    /// Other nodes currently visible as peers.
    pub peers: u64,
    /// Completed requests this node's own GPU answered.
    ///
    /// From `routing_metrics.pressure`. Unlike the `*_attempts` counters these
    /// count finished requests rather than tries, so they are the honest basis
    /// for "this ran here" vs "this ran on someone else's machine".
    pub locally_served: u64,
    /// Completed requests a peer answered for this node — i.e. this machine
    /// CONSUMED another member's compute. The one true "I used someone else's
    /// hardware" figure available today.
    pub remotely_served: u64,
    /// Completed requests answered by a configured endpoint (not a mesh peer).
    pub endpoint_served: u64,
}

/// Pure extractor: project a raw SDK status payload into [`MeshServingUsage`].
///
/// Every field is read defensively (missing → 0) so an SDK shape change
/// degrades to "no usage shown" rather than an error. Kept pure so it can be
/// unit-tested against a captured payload without a live runtime.
pub fn serving_usage_from_payload(payload: &serde_json::Value) -> MeshServingUsage {
    let u64_at = |v: &serde_json::Value| v.as_u64().unwrap_or(0);
    let rm = payload.get("routing_metrics");
    let local = rm.and_then(|m| m.get("local_node"));
    let pressure = rm.and_then(|m| m.get("pressure"));
    let get_u64 = |obj: Option<&serde_json::Value>, key: &str| {
        obj.and_then(|o| o.get(key)).map(u64_at).unwrap_or(0)
    };
    MeshServingUsage {
        inflight: local
            .and_then(|l| l.get("current_inflight_requests"))
            .map(u64_at)
            .or_else(|| payload.get("inflight_requests").map(u64_at))
            .unwrap_or(0),
        peak_inflight: get_u64(local, "peak_inflight_requests"),
        requests_served: get_u64(rm, "request_count"),
        tokens_served: get_u64(rm, "completion_tokens_observed"),
        tokens_per_second: rm
            .and_then(|m| m.get("avg_tokens_per_second"))
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0),
        local_attempts: get_u64(local, "local_attempt_count"),
        remote_attempts: get_u64(local, "remote_attempt_count"),
        endpoint_attempts: get_u64(local, "endpoint_attempt_count"),
        peers: payload
            .get("peers")
            .and_then(serde_json::Value::as_array)
            .map(|a| a.len() as u64)
            .unwrap_or(0),
        locally_served: get_u64(pressure, "locally_served_request_count"),
        remotely_served: get_u64(pressure, "remotely_served_request_count"),
        endpoint_served: get_u64(pressure, "endpoint_request_count"),
    }
}
