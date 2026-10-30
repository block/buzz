//! The real [`Substrate`]: kube-rs against a live apiserver.
//!
//! Everything that *decides* lives in `classify`/`gc`; this module only
//! performs I/O and maps apiserver responses onto the trait's vocabulary.
//! Three mappings here are normative rather than incidental:
//!
//! * **409 is discriminated on `Status.reason`, never on the code.** A create
//!   409 is `AlreadyExists`; a delete 409 from a failed precondition is
//!   `Conflict`. Branching on `code == 409` conflates a lost create race with
//!   a stale fence and is the trap the spec names (`:780-794`).
//! * **Reads leave `resourceVersion` unset**, which is the quorum read. `"0"`
//!   is the cache read, and a confirmed absence from a cache is proof of
//!   nothing (`:761-769`).
//! * **Deletes never set `grace_period_seconds`**, so the object's own 60s
//!   budget applies. Passing `0` is a force-kill that discards the shutdown
//!   window the pod declares (`:1185-1189`).

use crate::classify::Fence;
use crate::reconcile::{CreateOutcome, DeleteOutcome, Substrate};
use chrono::{DateTime, Utc};
use k8s_openapi::api::core::v1::{Namespace, Pod, Secret};
use kube::api::{Api, DeleteParams, GetParams, ListParams, PostParams, Preconditions};
use kube::core::ErrorResponse;
use kube::{Client, Resource};
use std::time::{Duration, Instant};

/// `Status.reason` values we branch on. Spelled once so the two 409 arms are
/// visibly the same discriminator read two ways.
///
/// These are wire strings `apimachinery` chooses, not names this crate picks:
/// `StatusReasonAlreadyExists`, `StatusReasonConflict`, `StatusReasonNotFound`,
/// and `StatusReasonForbidden` in `k8s.io/apimachinery/pkg/apis/meta/v1/types.go`.
/// kube-core types `ErrorResponse::reason` as a bare `String`, so there is no
/// upstream constant to bind to and the spelling is pinned by test instead.
const REASON_ALREADY_EXISTS: &str = "AlreadyExists";
const REASON_CONFLICT: &str = "Conflict";
const REASON_NOT_FOUND: &str = "NotFound";
const REASON_FORBIDDEN: &str = "Forbidden";

/// The apiserver-backed substrate for one deploy operation.
pub struct Cluster {
    client: Client,
    namespace: String,
    /// Start of *this operation*, for the deadline. Monotonic: the 600s budget
    /// must not move when the wall clock does.
    started: Instant,
}

/// The typed API error underneath a `kube::Error`, if it is one.
fn api_error(error: &kube::Error) -> Option<&ErrorResponse> {
    match error {
        kube::Error::Api(response) => Some(response),
        _ => None,
    }
}

/// Does this error carry the given `Status.reason`?
fn reason_is(error: &kube::Error, reason: &str) -> bool {
    api_error(error).is_some_and(|e| e.reason == reason)
}

impl Cluster {
    pub fn new(client: Client, namespace: &str) -> Self {
        Self {
            client,
            namespace: namespace.to_string(),
            started: Instant::now(),
        }
    }

    fn pods(&self) -> Api<Pod> {
        Api::namespaced(self.client.clone(), &self.namespace)
    }

    fn secrets(&self) -> Api<Secret> {
        Api::namespaced(self.client.clone(), &self.namespace)
    }

    /// List an object kind through the raw client so the response's HTTP
    /// `Date` header is reachable.
    ///
    /// `Api::list` returns only the decoded body, and the apiserver's clock is
    /// the *only* clock the orphan-Secret age gate may use — a desktop's local
    /// clock running fast computes every in-flight Secret as expired
    /// (`:1321-1335`). So the list goes through `Client::send`, which hands
    /// back the whole `http::Response`.
    async fn list_with_date<K>(
        &self,
        selector: &str,
    ) -> Result<(Vec<K>, Option<DateTime<Utc>>), String>
    where
        K: Resource<Scope = k8s_openapi::NamespaceResourceScope>
            + Clone
            + serde::de::DeserializeOwned
            + std::fmt::Debug,
        K::DynamicType: Default,
    {
        let dt = K::DynamicType::default();
        let url = K::url_path(&dt, Some(&self.namespace));
        // resourceVersion deliberately unset: quorum read.
        let params = ListParams {
            label_selector: Some(selector.to_string()),
            ..Default::default()
        };
        let request = kube::core::Request::new(url)
            .list(&params)
            .map_err(|e| format!("could not build a list request: {e}"))?;
        let (parts, body) = request.into_parts();
        let response = self
            .client
            .send(http::Request::from_parts(parts, body.into()))
            .await
            .map_err(|e| format!("could not list {}: {e}", K::plural(&dt)))?;

        // Parsed before the body is consumed, and independently of it: a
        // missing or malformed header is not a list failure, it just means the
        // orphan sweep has no clock and skips.
        let server_now = response
            .headers()
            .get(http::header::DATE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| DateTime::parse_from_rfc2822(v).ok())
            .map(|v| v.with_timezone(&Utc));

        let bytes = http_body_util::BodyExt::collect(response.into_body())
            .await
            .map_err(|e| format!("could not read the {} list body: {e}", K::plural(&dt)))?
            .to_bytes();
        let list: kube::core::ObjectList<K> = serde_json::from_slice(&bytes)
            .map_err(|e| format!("could not decode the {} list: {e}", K::plural(&dt)))?;

        Ok((list.items, server_now))
    }
}

impl Substrate for Cluster {
    async fn ensure_namespace(&self, namespace: &str) -> Result<(), String> {
        let api: Api<Namespace> = Api::all(self.client.clone());
        if api
            .get_opt(namespace)
            .await
            .map_err(|e| format!("could not check whether namespace {namespace} exists: {e}"))?
            .is_some()
        {
            return Ok(());
        }

        let spec = Namespace {
            metadata: kube::core::ObjectMeta {
                name: Some(namespace.to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        match api.create(&PostParams::default(), &spec).await {
            Ok(_) => Ok(()),
            // Someone else created it between our check and our create. That
            // is the desired end state, not a failure.
            Err(e) if reason_is(&e, REASON_ALREADY_EXISTS) => Ok(()),
            // Namespace-create is frequently denied on shared clusters. Name
            // the exact command an operator runs, and never silently fall back
            // to `default` — deploying an agent into someone else's namespace
            // is worse than refusing (`:1002-1005`).
            Err(e) if reason_is(&e, REASON_FORBIDDEN) => Err(format!(
                "not authorized to create namespace {namespace}. Ask a cluster \
                 administrator to run `kubectl create namespace {namespace}`, \
                 then try again."
            )),
            Err(e) => Err(format!("could not create namespace {namespace}: {e}")),
        }
    }

    async fn list_pods(&self, selector: &str) -> Result<(Vec<Pod>, Option<DateTime<Utc>>), String> {
        self.list_with_date::<Pod>(selector).await
    }

    async fn list_secrets(&self, selector: &str) -> Result<Vec<Secret>, String> {
        Ok(self.list_with_date::<Secret>(selector).await?.0)
    }

    async fn secret_exists(&self, name: &str) -> Result<bool, String> {
        // `GetParams::default()` leaves resourceVersion unset — the quorum
        // read this check requires to be proof of anything.
        match self.secrets().get_with(name, &GetParams::default()).await {
            Ok(_) => Ok(true),
            Err(e) if reason_is(&e, REASON_NOT_FOUND) => Ok(false),
            Err(e) => Err(format!("could not check whether secret {name} exists: {e}")),
        }
    }

    async fn create_secret(&self, secret: &Secret) -> Result<(), String> {
        let name = secret.metadata.name.clone().unwrap_or_default();
        self.secrets()
            .create(&PostParams::default(), secret)
            .await
            .map(|_| ())
            .map_err(|e| format!("could not create secret {name}: {e}"))
    }

    async fn create_pod(&self, pod: &Pod) -> Result<CreateOutcome, String> {
        let name = pod.metadata.name.clone().unwrap_or_default();
        match self.pods().create(&PostParams::default(), pod).await {
            Ok(_) => Ok(CreateOutcome::Created),
            // The deterministic name is taken: a concurrent attempt won the
            // election. Discriminated on the reason — a 409 whose reason is
            // `Conflict` is a different condition and must not be read as a
            // lost race.
            Err(e) if reason_is(&e, REASON_ALREADY_EXISTS) => Ok(CreateOutcome::AlreadyExists),
            Err(e) => Err(format!("could not create pod {name}: {e}")),
        }
    }

    async fn delete_pod(&self, name: &str, fence: &Fence) -> Result<DeleteOutcome, String> {
        let params = DeleteParams {
            preconditions: Some(Preconditions {
                uid: Some(fence.uid.clone()),
                resource_version: Some(fence.resource_version.clone()),
            }),
            // grace_period_seconds deliberately unset: the pod's own 60s
            // budget applies.
            ..Default::default()
        };
        match self.pods().delete(name, &params).await {
            Ok(_) => Ok(DeleteOutcome::Accepted),
            Err(e) if reason_is(&e, REASON_NOT_FOUND) => Ok(DeleteOutcome::NotFound),
            // The object changed since the observation that authorized this
            // delete. Same HTTP code as the create race above, different
            // reason, different meaning.
            Err(e) if reason_is(&e, REASON_CONFLICT) => Ok(DeleteOutcome::PreconditionFailed),
            Err(e) => Err(format!("could not delete pod {name}: {e}")),
        }
    }

    async fn delete_secret(&self, name: &str) -> Result<(), String> {
        match self.secrets().delete(name, &DeleteParams::default()).await {
            Ok(_) => Ok(()),
            // Already gone is the desired end state.
            Err(e) if reason_is(&e, REASON_NOT_FOUND) => Ok(()),
            Err(e) => Err(format!("could not delete secret {name}: {e}")),
        }
    }

    async fn get_pod(&self, name: &str) -> Result<Option<Pod>, String> {
        match self.pods().get_with(name, &GetParams::default()).await {
            Ok(pod) => Ok(Some(pod)),
            Err(e) if reason_is(&e, REASON_NOT_FOUND) => Ok(None),
            Err(e) => Err(format!("could not read pod {name}: {e}")),
        }
    }

    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }

    fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::Response;
    use kube::client::Body;
    use std::sync::{Arc, Mutex};
    use tower::service_fn;

    fn list_response(date: Option<&str>) -> Response<Body> {
        let mut response = Response::builder().status(200);
        if let Some(date) = date {
            response = response.header(http::header::DATE, date);
        }
        response
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({
                    "apiVersion": "v1",
                    "kind": "PodList",
                    "metadata": {"resourceVersion": "17"},
                    "items": [{
                        "apiVersion": "v1",
                        "kind": "Pod",
                        "metadata": {"name": "sprig"},
                        "spec": {"containers": [{"name": "agent", "image": "example.invalid/sprig@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}]}
                    }]
                }))
                .unwrap(),
            ))
            .unwrap()
    }

    async fn list_through_real_request_path(
        date: Option<&'static str>,
    ) -> (Vec<Pod>, Option<DateTime<Utc>>, String) {
        let observed_uri = Arc::new(Mutex::new(None));
        let service_uri = Arc::clone(&observed_uri);
        let service = service_fn(move |request: http::Request<Body>| {
            let service_uri = Arc::clone(&service_uri);
            async move {
                *service_uri.lock().unwrap() = Some(request.uri().to_string());
                Ok::<_, std::convert::Infallible>(list_response(date))
            }
        });
        let cluster = Cluster::new(Client::new(service, "ignored"), "owned-ns");
        let result = cluster
            .list_with_date::<Pod>("app.kubernetes.io/managed-by=buzz-backend-kubernetes")
            .await
            .unwrap();
        let uri = observed_uri.lock().unwrap().take().unwrap();
        (result.0, result.1, uri)
    }

    /// Exercise the shipped `Request` + `Client::send` seam. A fake
    /// reconciler would not prove that kube-rs emits a quorum list request or
    /// that the apiserver's clock survives body decoding.
    #[tokio::test]
    async fn list_with_date_uses_a_quorum_request_and_returns_the_server_clock() {
        let (pods, server_now, uri) =
            list_through_real_request_path(Some("Sun, 02 Aug 2026 04:00:00 GMT")).await;

        assert_eq!(pods.len(), 1, "fixture must contain one decoded pod");
        assert_eq!(pods[0].metadata.name.as_deref(), Some("sprig"));
        assert!(uri.starts_with("/api/v1/namespaces/owned-ns/pods?"));
        assert!(
            uri.contains("labelSelector=app.kubernetes.io%2Fmanaged-by%3Dbuzz-backend-kubernetes")
        );
        assert!(
            !uri.contains("resourceVersion"),
            "cache read leaked into {uri}"
        );
        assert_eq!(
            server_now.unwrap().to_rfc3339(),
            "2026-08-02T04:00:00+00:00"
        );
    }

    /// Header failure is deliberately not list failure: without a trustworthy
    /// apiserver clock the orphan sweep skips, but normal reconciliation still
    /// receives the decoded objects.
    #[tokio::test]
    async fn list_with_date_keeps_items_when_the_server_clock_is_unusable() {
        for date in [Some("not a date"), None] {
            let (pods, server_now, _) = list_through_real_request_path(date).await;
            assert_eq!(pods.len(), 1, "fixture must contain one decoded pod");
            assert!(server_now.is_none(), "unexpected clock for {date:?}");
        }
    }

    /// A typed apiserver error, as kube-rs surfaces it.
    fn api(reason: &str, code: u16) -> kube::Error {
        kube::Error::Api(ErrorResponse {
            status: "Failure".into(),
            message: String::new(),
            reason: reason.into(),
            code,
        })
    }

    /// The one discriminator the whole file rests on, and the trap the spec
    /// predicts: "an implementation that branches on the code alone will
    /// eventually take the adoption path on a failed delete or vice versa"
    /// (`:788-790`).
    ///
    /// Both of these are 409. Reading the *code* makes them identical; reading
    /// `Status.reason` keeps a lost create race and a stale fence apart. The
    /// mutation that must fail this test is `e.reason == …` → `e.code == 409`,
    /// which no other test in the crate would catch — the fakes never produce
    /// a real `kube::Error`.
    #[test]
    fn the_two_409s_are_never_conflated() {
        let already_exists = api(REASON_ALREADY_EXISTS, 409);
        let conflict = api(REASON_CONFLICT, 409);

        assert!(reason_is(&already_exists, REASON_ALREADY_EXISTS));
        assert!(reason_is(&conflict, REASON_CONFLICT));
        // The cross terms are the whole point.
        assert!(!reason_is(&already_exists, REASON_CONFLICT));
        assert!(!reason_is(&conflict, REASON_ALREADY_EXISTS));
    }

    /// A transport-level failure is not an apiserver verdict. It must fall
    /// through to the error arm rather than being read as any reason — a
    /// connection reset silently classified as `NotFound` would report a pod
    /// as confirmed-absent, which the classifier treats as proof.
    #[test]
    fn a_non_api_error_carries_no_reason() {
        let transport = kube::Error::LinesCodecMaxLineLengthExceeded;
        assert!(api_error(&transport).is_none());
        for reason in [
            REASON_ALREADY_EXISTS,
            REASON_CONFLICT,
            REASON_NOT_FOUND,
            REASON_FORBIDDEN,
        ] {
            assert!(!reason_is(&transport, reason), "matched {reason}");
        }
    }

    /// `reason` is `#[serde(default)]` in kube-core, so an apiserver that
    /// omits it yields an empty string. That must match nothing rather than
    /// matching an empty pattern by accident.
    #[test]
    fn an_absent_reason_matches_nothing() {
        let bare = api("", 409);
        assert!(!reason_is(&bare, REASON_ALREADY_EXISTS));
        assert!(!reason_is(&bare, REASON_CONFLICT));
    }

    /// The consts are the apiserver's spelling, asserted against literals
    /// rather than against themselves.
    ///
    /// Every other test here references the consts symbolically on both sides
    /// — fixture *and* assertion — which is true for any pair of distinct
    /// values. That tests the discriminator is self-consistent, not that it is
    /// correct: swapping the two 409 values inverts `AlreadyExists` and
    /// `Conflict` at a real apiserver (`:788-790`'s failure, reached by
    /// editing a string instead of a branch) with every other test still
    /// green. These are `apimachinery`'s wire strings and kube-core exposes no
    /// constant for them, so a literal is the only external anchor available.
    /// Found by Quinn's mutation matrix; M2/M3/M4 survived without it.
    #[test]
    fn the_reason_consts_are_the_apiservers_spelling() {
        assert_eq!(REASON_ALREADY_EXISTS, "AlreadyExists");
        assert_eq!(REASON_CONFLICT, "Conflict");
        assert_eq!(REASON_NOT_FOUND, "NotFound");
        assert_eq!(REASON_FORBIDDEN, "Forbidden");
    }
}
