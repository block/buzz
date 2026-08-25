//! Google Cloud Storage provider, backed by Google's official client.
//!
//! This is the only module in Buzz that knows what an object *generation* is.
//! Everything above the seam holds a [`Revision`] and cannot tell a generation
//! from an S3 ETag.
//!
//! ## Why generations, not ETags
//!
//! Cloud Storage stamps every object write with a monotonically increasing
//! `generation`, and accepts it back as the `ifGenerationMatch` precondition.
//! That is a first-class compare-and-swap token: `ifGenerationMatch=0` means
//! "commit only if the object does not exist", and `ifGenerationMatch=<g>`
//! means "commit only if the object is still at `<g>`". A stale precondition
//! is refused with HTTP 412, which this module reports as
//! [`ConditionalWrite::Conflict`] — an ordinary lost race, never a backend
//! error.
//!
//! ## Bucket contract, checked at construction
//!
//! The S3 provider detects bucket versioning empirically, by writing a probe
//! object and looking for a version id on the response. That heuristic is
//! meaningless here: *every* Cloud Storage object carries a generation whether
//! or not the bucket retains old ones. [`GcsObjectStore::connect`] reads the
//! bucket's metadata instead and refuses to build a client unless object
//! versioning is off **and** soft-delete retention is zero. Both settings would
//! otherwise leave a restorable copy behind after a delete, so a deletion
//! request could report success while the bytes remain reachable. Checking at
//! construction also catches out-of-band configuration drift on every boot.
//!
//! ## Retries
//!
//! The client's own retry loop is disabled ([`no_client_retries`]) and this
//! module owns one bounded policy instead, so that three rules hold visibly:
//!
//! - a conditional write is only ever retried carrying its exact original
//!   precondition — it is never downgraded to an unconditional write;
//! - HTTP 429 is throttling, never evidence of a lost race, so it paces the
//!   caller and, if the budget runs out, surfaces as
//!   [`ObjectStoreError::Throttled`] for the caller to absorb as backpressure;
//! - when an attempt fails without a classified answer, a subsequent 412 is
//!   *not* assumed to be someone else's commit. The object is reread and the
//!   committed body decides — see [`GcsObjectStore::put_conditional`].
//!
//! The single exception is [`ObjectStore::put_file`], which streams a
//! multi-hundred-megabyte media blob: it delegates to the client's resumable
//! upload retry so a transient failure resumes mid-object instead of
//! restarting the transfer.

use std::future::Future;
use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::StreamExt;
use google_cloud_gax::error::Error as GcsError;
use google_cloud_gax::retry_policy::RetryPolicyExt;
use google_cloud_storage::client::{Storage, StorageControl};
use google_cloud_storage::model_ext::ReadRange;
use google_cloud_storage::retry_policy::RetryableErrors;

use crate::error::ObjectStoreError;
use crate::revision::{ConditionalWrite, ProviderKind, Revision, WriteCondition};
use crate::{
    BulkDeleteOutcome, ByteStream, ImmutableWrite, ListPage, ObjectMeta, ObjectStore,
    ObjectVersionEntry, ObjectVersionKind, ObjectVersionRef, ObjectVersionsPage,
};

/// How many deletes a bulk delete keeps in flight.
///
/// Cloud Storage has no batch-delete RPC, so a bulk delete is N individual
/// deletes. The width is bounded to keep one caller from consuming the whole
/// per-bucket request budget and throttling every other operation.
const BULK_DELETE_CONCURRENCY: usize = 12;

/// Upper bound on a provider-advertised `Retry-After`.
///
/// Honouring the header is required, but an unbounded sleep would let one
/// response stall a request for minutes.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(30);

/// Longest provider detail kept on an error.
///
/// Cloud Storage error bodies are JSON diagnostics, but they can echo request
/// detail; the message is bounded so an error can never grow into a log of the
/// request.
const MAX_ERROR_DETAIL: usize = 512;

/// Bounded retry policy for one [`GcsObjectStore`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcsRetryConfig {
    /// Total attempts, including the first. `1` disables retries.
    pub max_attempts: u32,
    /// Backoff ceiling used for the first retry, doubled thereafter.
    pub initial_backoff: Duration,
    /// Ceiling on any single computed backoff.
    pub max_backoff: Duration,
}

impl Default for GcsRetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 6,
            initial_backoff: Duration::from_millis(200),
            max_backoff: Duration::from_secs(8),
        }
    }
}

/// Connection inputs for [`GcsObjectStore::connect`].
#[derive(Debug, Clone)]
pub struct GcsStoreConfig {
    /// Bucket name, without any `gs://` or resource-path decoration.
    pub bucket: String,
    /// Bounded retry policy for this client.
    pub retry: GcsRetryConfig,
}

impl GcsStoreConfig {
    /// Configure a bucket with the default retry policy.
    pub fn new(bucket: impl Into<String>) -> Self {
        Self {
            bucket: bucket.into(),
            retry: GcsRetryConfig::default(),
        }
    }
}

/// The two bucket settings that decide whether a delete proves absence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BucketContract {
    versioning_enabled: bool,
    soft_delete_retention: Duration,
}

impl BucketContract {
    fn of(bucket: &google_cloud_storage::model::Bucket) -> Self {
        let versioning_enabled = bucket.versioning.as_ref().is_some_and(|v| v.enabled);
        let soft_delete_retention = bucket
            .soft_delete_policy
            .as_ref()
            .and_then(|policy| policy.retention_duration.as_ref())
            .map(|d| {
                let seconds = u64::try_from(d.seconds()).unwrap_or(0);
                let nanos = u32::try_from(d.nanos()).unwrap_or(0);
                Duration::new(seconds, nanos)
            })
            .unwrap_or(Duration::ZERO);
        Self {
            versioning_enabled,
            soft_delete_retention,
        }
    }

    /// Whether a delete on this bucket leaves a restorable copy behind.
    ///
    /// Object versioning and soft delete are different mechanisms with the
    /// same consequence for Buzz: the deleted bytes stay reachable, so a
    /// deletion cannot claim absence. Both therefore answer the seam's
    /// "does this bucket retain non-current versions" question with yes.
    fn retains_noncurrent_versions(&self) -> bool {
        self.versioning_enabled || !self.soft_delete_retention.is_zero()
    }

    /// Fail closed unless the bucket satisfies the deletion contract.
    fn admit(&self, bucket: &str) -> Result<(), ObjectStoreError> {
        let mut violations = Vec::new();
        if self.versioning_enabled {
            violations.push("object versioning is enabled".to_string());
        }
        if !self.soft_delete_retention.is_zero() {
            violations.push(format!(
                "soft-delete retention is {}s (must be 0)",
                self.soft_delete_retention.as_secs()
            ));
        }
        if violations.is_empty() {
            return Ok(());
        }
        Err(ObjectStoreError::Config(format!(
            "bucket {bucket} does not satisfy the deletion contract ({}): a delete would leave a \
             restorable copy behind, so deletion could report success while the object stays \
             reachable",
            violations.join("; ")
        )))
    }
}

/// Google Cloud Storage client, authenticated with Application Default
/// Credentials.
///
/// Credentials are never configured in code: the client resolves them through
/// ADC (attached service account on GCE/GKE/Cloud Run, workload identity, or a
/// developer's `gcloud` credentials). There is no key material, HMAC pair, or
/// key file on this path.
pub struct GcsObjectStore {
    bucket: String,
    /// `projects/_/buckets/<bucket>`, the resource name both clients address.
    resource: String,
    data: Storage,
    control: StorageControl,
    retry: GcsRetryConfig,
}

/// Retry policy handed to the client so it performs exactly one attempt.
///
/// This module owns retries (see the module docs); a second, invisible loop
/// underneath would multiply attempts and hide throttling from the caller.
fn no_client_retries() -> impl google_cloud_gax::retry_policy::RetryPolicy {
    RetryableErrors.with_attempt_limit(1)
}

impl GcsObjectStore {
    /// Build a client against a bucket and verify its deletion contract.
    ///
    /// Fails closed when object versioning is enabled or soft-delete retention
    /// is non-zero.
    pub async fn connect(config: &GcsStoreConfig) -> Result<Self, ObjectStoreError> {
        if config.bucket.is_empty() {
            return Err(ObjectStoreError::Config(
                "gcs bucket must be configured".to_string(),
            ));
        }
        if config.retry.max_attempts == 0 {
            return Err(ObjectStoreError::Config(
                "gcs retry max_attempts must be at least 1".to_string(),
            ));
        }

        let data = Storage::builder()
            .with_retry_policy(no_client_retries())
            .build()
            .await
            .map_err(|e| ObjectStoreError::Config(format!("gcs storage client: {e}")))?;
        let control = StorageControl::builder()
            .with_retry_policy(no_client_retries())
            .build()
            .await
            .map_err(|e| ObjectStoreError::Config(format!("gcs storage control client: {e}")))?;

        let store = Self {
            resource: bucket_resource(&config.bucket),
            bucket: config.bucket.clone(),
            data,
            control,
            retry: config.retry,
        };
        store.read_bucket_contract().await?.admit(&store.bucket)?;
        Ok(store)
    }

    /// The bucket this client addresses.
    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    /// Read the bucket's versioning and soft-delete settings.
    async fn read_bucket_contract(&self) -> Result<BucketContract, ObjectStoreError> {
        let bucket = self
            .with_retries("get_bucket", &self.bucket, || async {
                self.control
                    .get_bucket()
                    .set_name(self.resource.clone())
                    .send()
                    .await
                    .map_err(|e| classify("get_bucket", &self.bucket, e))
            })
            .await?;
        Ok(BucketContract::of(&bucket))
    }

    /// Run `attempt` under this client's bounded retry policy.
    ///
    /// Only throttling, transient backend answers, and unclassified transport
    /// failures are retried; everything else is the caller's answer on the
    /// first attempt. Conditional writes do not use this helper — they need
    /// per-attempt bookkeeping and have their own loop.
    async fn with_retries<T, F, Fut>(
        &self,
        operation: &'static str,
        key: &str,
        mut attempt: F,
    ) -> Result<T, ObjectStoreError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, ObjectStoreError>>,
    {
        for attempt_index in 0..self.retry.max_attempts {
            match attempt().await {
                Ok(value) => return Ok(value),
                Err(error) => match self.retry_delay(&error, attempt_index) {
                    Some(delay) if attempt_index + 1 < self.retry.max_attempts => {
                        tracing::debug!(
                            provider = "gcs",
                            operation,
                            attempt = attempt_index + 1,
                            delay_ms = delay.as_millis() as u64,
                            "retrying object store operation"
                        );
                        tokio::time::sleep(delay).await;
                    }
                    _ => return Err(error),
                },
            }
        }
        // Unreachable while `max_attempts >= 1`, which the constructor enforces.
        Err(ObjectStoreError::TransportRetryable {
            operation,
            message: format!("retry budget exhausted for {key:?}"),
        })
    }

    /// How long to wait before retrying, or `None` when the error is final.
    fn retry_delay(&self, error: &ObjectStoreError, attempt_index: u32) -> Option<Duration> {
        match error {
            ObjectStoreError::Throttled { retry_after, .. } => Some(
                retry_after
                    .map(|hint| hint.min(MAX_RETRY_AFTER))
                    .unwrap_or_else(|| self.backoff(attempt_index)),
            ),
            ObjectStoreError::TransportRetryable { .. }
            | ObjectStoreError::TransportAmbiguous { .. } => Some(self.backoff(attempt_index)),
            _ => None,
        }
    }

    /// Capped exponential backoff with full jitter.
    ///
    /// Full jitter (a uniform draw from `[1ms, cap]`) rather than the raw
    /// exponent: a hot pointer is written by several racers at once, and
    /// unjittered backoff would keep them synchronised into the same retry
    /// instants.
    fn backoff(&self, attempt_index: u32) -> Duration {
        let cap = self
            .retry
            .initial_backoff
            .saturating_mul(1u32 << attempt_index.min(16))
            .min(self.retry.max_backoff);
        let cap_ms = u64::try_from(cap.as_millis()).unwrap_or(u64::MAX).max(1);
        Duration::from_millis(1 + rand::random::<u64>() % cap_ms)
    }

    /// One conditional-write attempt, carrying `precondition` verbatim.
    async fn write_once(
        &self,
        operation: &'static str,
        key: &str,
        bytes: &[u8],
        content_type: &str,
        precondition: Option<i64>,
    ) -> Result<Revision, ObjectStoreError> {
        let mut write = self
            .data
            .write_object(
                self.resource.clone(),
                key.to_string(),
                Bytes::copy_from_slice(bytes),
            )
            .set_content_type(content_type.to_string());
        if let Some(generation) = precondition {
            write = write.set_if_generation_match(generation);
        }
        write
            .send_unbuffered()
            .await
            .map(|object| Revision::GcsGeneration(object.generation))
            .map_err(|e| classify(operation, key, e))
    }

    /// After an unclassified failure, let the stored object decide whether the
    /// write committed.
    ///
    /// A 412 arriving after an attempt whose outcome was never classified is
    /// genuinely ambiguous: either another writer won the race, or *our own*
    /// earlier attempt committed and the retry then found its own generation in
    /// place. Guessing either way is a correctness bug, so the object is reread
    /// and its body answers. Bodies are compared rather than generations
    /// because the generation the winning attempt would have returned was never
    /// received.
    async fn classify_ambiguous_commit(
        &self,
        key: &str,
        written: &[u8],
    ) -> Result<ConditionalWrite, ObjectStoreError> {
        match self.get_with_revision(key).await? {
            Some((revision, body)) if body.as_ref() == written => {
                Ok(ConditionalWrite::Committed(revision))
            }
            _ => Ok(ConditionalWrite::Conflict),
        }
    }
}

/// The resource name both Cloud Storage clients address a bucket by.
fn bucket_resource(bucket: &str) -> String {
    format!("projects/_/buckets/{bucket}")
}

/// The `ifGenerationMatch` value implementing a [`WriteCondition`].
///
/// `0` is Cloud Storage's create-only precondition: no live generation can be
/// zero, so it holds exactly when the object is absent.
fn generation_precondition(condition: &WriteCondition) -> Result<i64, ObjectStoreError> {
    match condition {
        WriteCondition::Absent => Ok(0),
        WriteCondition::Matches(revision) => revision.expect_gcs_generation(),
    }
}

/// Parse a `Retry-After` delta-seconds value, if the response carried one.
///
/// Only the delta-seconds form is read. Cloud Storage does not send the
/// HTTP-date form, and misreading a date as a duration would produce an absurd
/// sleep; an unparsed header simply falls back to computed backoff.
fn retry_after_of(error: &GcsError) -> Option<Duration> {
    error
        .http_headers()?
        .get("retry-after")?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

/// Bound and tidy a provider error for inclusion in an [`ObjectStoreError`].
fn detail(error: &GcsError) -> String {
    let mut message = error.to_string();
    if message.len() > MAX_ERROR_DETAIL {
        message.truncate(MAX_ERROR_DETAIL);
        message.push('…');
    }
    message
}

/// Map a client failure into the provider-neutral taxonomy.
///
/// The ordering is the whole point: a response that carried a status told us
/// something about the object, and stays a *classified* observation no matter
/// how the transport behaved afterwards. Only a failure with no status at all
/// becomes [`ObjectStoreError::TransportAmbiguous`], which is the set the Git
/// conformance probe drops from its observers. Widening it would let a real
/// conformance failure disappear.
fn classify(operation: &'static str, key: &str, error: GcsError) -> ObjectStoreError {
    let message = detail(&error);

    if let Some(status) = error.http_status_code() {
        return match status {
            404 => ObjectStoreError::NotFound { key: key.into() },
            412 => ObjectStoreError::Conflict { key: key.into() },
            429 => ObjectStoreError::Throttled {
                operation,
                retry_after: retry_after_of(&error),
            },
            408 | 500..=599 => ObjectStoreError::TransportRetryable { operation, message },
            _ => ObjectStoreError::Provider { operation, message },
        };
    }

    // gRPC-shaped answers carry an RPC status instead of an HTTP one. They are
    // still answers, so they classify the same way.
    if let Some(code) = error.status().map(|status| status.code) {
        use google_cloud_gax::error::rpc::Code;
        return match code {
            Code::NotFound => ObjectStoreError::NotFound { key: key.into() },
            Code::Aborted | Code::FailedPrecondition | Code::AlreadyExists => {
                ObjectStoreError::Conflict { key: key.into() }
            }
            Code::ResourceExhausted => ObjectStoreError::Throttled {
                operation,
                retry_after: retry_after_of(&error),
            },
            Code::Unavailable | Code::Internal | Code::DeadlineExceeded => {
                ObjectStoreError::TransportRetryable { operation, message }
            }
            _ => ObjectStoreError::Provider { operation, message },
        };
    }

    if error.is_connect() || error.is_io() || error.is_timeout() || error.is_transport() {
        return ObjectStoreError::TransportAmbiguous { operation, message };
    }

    // The client gave up before any attempt was classified, so the outcome of
    // the last one is unknown.
    if error.is_exhausted() {
        return ObjectStoreError::TransportAmbiguous { operation, message };
    }

    ObjectStoreError::Provider { operation, message }
}

/// Whether a failed attempt might still have reached the backend.
///
/// A refused connection never delivered the request, so a later 412 cannot be
/// this writer's own commit. Any other unclassified failure could have been a
/// response that was lost on the way back.
fn may_have_committed(error: &ObjectStoreError) -> bool {
    match error {
        ObjectStoreError::TransportAmbiguous { message, .. } => !is_connect_refusal(message),
        ObjectStoreError::TransportRetryable { .. } | ObjectStoreError::Throttled { .. } => false,
        _ => false,
    }
}

/// Whether an ambiguous failure's detail describes a connection that was never
/// established.
fn is_connect_refusal(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("connection refused")
        || message.contains("dns error")
        || message.contains("failed to lookup address")
}

#[async_trait]
impl ObjectStore for GcsObjectStore {
    fn provider(&self) -> ProviderKind {
        ProviderKind::Gcs
    }

    async fn put(
        &self,
        key: &str,
        bytes: &[u8],
        content_type: &str,
    ) -> Result<(), ObjectStoreError> {
        self.with_retries("put", key, || {
            self.write_once("put", key, bytes, content_type, None)
        })
        .await
        .map(|_| ())
    }

    async fn put_file(
        &self,
        key: &str,
        path: &Path,
        content_type: &str,
    ) -> Result<(), ObjectStoreError> {
        let file = tokio::fs::File::open(path)
            .await
            .map_err(|e| ObjectStoreError::Provider {
                operation: "put_file",
                message: e.to_string(),
            })?;

        // The one operation that keeps the client's own retry loop: a media
        // blob can be hundreds of megabytes, and the client's resumable upload
        // resumes mid-object where this module's loop would restart the whole
        // transfer. The upload is unconditional, so a retry cannot disturb any
        // precondition.
        self.data
            .write_object(self.resource.clone(), key.to_string(), file)
            .set_content_type(content_type.to_string())
            .with_retry_policy(
                RetryableErrors
                    .with_attempt_limit(self.retry.max_attempts)
                    .with_time_limit(Duration::from_secs(300)),
            )
            .send_unbuffered()
            .await
            .map_err(|e| classify("put_file", key, e))?;
        Ok(())
    }

    async fn put_immutable(
        &self,
        key: &str,
        bytes: &[u8],
        content_type: &str,
    ) -> Result<ImmutableWrite, ObjectStoreError> {
        match self
            .put_conditional(key, bytes, content_type, WriteCondition::Absent)
            .await?
        {
            ConditionalWrite::Committed(_) => Ok(ImmutableWrite::Created),
            ConditionalWrite::Conflict => Ok(ImmutableWrite::AlreadyPresent),
        }
    }

    async fn put_conditional(
        &self,
        key: &str,
        bytes: &[u8],
        content_type: &str,
        condition: WriteCondition,
    ) -> Result<ConditionalWrite, ObjectStoreError> {
        let precondition = generation_precondition(&condition)?;
        // Set once an attempt fails without a classified answer. From then on a
        // 412 is no longer self-evidently someone else's commit.
        let mut outcome_unknown = false;

        for attempt_index in 0..self.retry.max_attempts {
            let error = match self
                .write_once(
                    "put_conditional",
                    key,
                    bytes,
                    content_type,
                    Some(precondition),
                )
                .await
            {
                Ok(revision) => return Ok(ConditionalWrite::Committed(revision)),
                Err(ObjectStoreError::Conflict { .. }) if !outcome_unknown => {
                    return Ok(ConditionalWrite::Conflict);
                }
                Err(ObjectStoreError::Conflict { .. }) => {
                    return self.classify_ambiguous_commit(key, bytes).await;
                }
                Err(error) => error,
            };

            outcome_unknown |= may_have_committed(&error);

            // Retrying always replays `precondition` verbatim: the loop never
            // relaxes a compare-and-swap into a blind overwrite.
            match self.retry_delay(&error, attempt_index) {
                Some(delay) if attempt_index + 1 < self.retry.max_attempts => {
                    tracing::debug!(
                        provider = "gcs",
                        operation = "put_conditional",
                        attempt = attempt_index + 1,
                        delay_ms = delay.as_millis() as u64,
                        outcome_unknown,
                        "retrying conditional write with its original precondition"
                    );
                    tokio::time::sleep(delay).await;
                }
                _ => return Err(error),
            }
        }

        Err(ObjectStoreError::TransportRetryable {
            operation: "put_conditional",
            message: "retry budget exhausted".to_string(),
        })
    }

    async fn get(&self, key: &str) -> Result<Bytes, ObjectStoreError> {
        self.with_retries("get", key, || async {
            let mut response = self
                .data
                .read_object(self.resource.clone(), key.to_string())
                .send()
                .await
                .map_err(|e| classify("get", key, e))?;
            let mut body = Vec::with_capacity(response.object().size.max(0) as usize);
            while let Some(chunk) = response.next().await {
                body.extend_from_slice(&chunk.map_err(|e| classify("get", key, e))?);
            }
            Ok(Bytes::from(body))
        })
        .await
    }

    async fn get_range(&self, key: &str, start: u64, end: u64) -> Result<Bytes, ObjectStoreError> {
        if end < start {
            return Err(ObjectStoreError::Provider {
                operation: "get_range",
                message: format!("inverted byte range {start}..={end}"),
            });
        }
        // The seam's range is inclusive on both ends; Cloud Storage takes an
        // offset and a length.
        let length = end - start + 1;

        self.with_retries("get_range", key, || async {
            let mut response = self
                .data
                .read_object(self.resource.clone(), key.to_string())
                .set_read_range(ReadRange::segment(start, length))
                .send()
                .await
                .map_err(|e| classify("get_range", key, e))?;
            let mut body = Vec::new();
            while let Some(chunk) = response.next().await {
                body.extend_from_slice(&chunk.map_err(|e| classify("get_range", key, e))?);
            }
            Ok(Bytes::from(body))
        })
        .await
    }

    async fn get_stream(&self, key: &str) -> Result<ByteStream, ObjectStoreError> {
        // Only opening the stream is retried. Once bytes are flowing the
        // caller owns the body, and restarting mid-response would silently
        // splice two reads together.
        let response = self
            .with_retries("get_stream", key, || async {
                self.data
                    .read_object(self.resource.clone(), key.to_string())
                    .send()
                    .await
                    .map_err(|e| classify("get_stream", key, e))
            })
            .await?;

        // `ReadObjectResponse` only exposes a `Stream` adapter behind the
        // client's `unstable-stream` feature, so the stream is unfolded from
        // its stable chunk-at-a-time API instead of opting into an unstable
        // surface.
        let key = key.to_string();
        Ok(Box::pin(futures_util::stream::unfold(
            response,
            move |mut response| {
                let key = key.clone();
                async move {
                    let chunk = response.next().await?;
                    Some((chunk.map_err(|e| classify("get_stream", &key, e)), response))
                }
            },
        )))
    }

    async fn get_with_revision(
        &self,
        key: &str,
    ) -> Result<Option<(Revision, Bytes)>, ObjectStoreError> {
        self.with_retries("get_with_revision", key, || async {
            let mut response = match self
                .data
                .read_object(self.resource.clone(), key.to_string())
                .send()
                .await
            {
                Ok(response) => response,
                Err(error) => {
                    return match classify("get_with_revision", key, error) {
                        ObjectStoreError::NotFound { .. } => Ok(None),
                        other => Err(other),
                    };
                }
            };

            // Body and generation both come off this one response, so the
            // revision a caller predicates its next write on always describes
            // the bytes it just read.
            let revision = Revision::GcsGeneration(response.object().generation);
            let mut body = Vec::with_capacity(response.object().size.max(0) as usize);
            while let Some(chunk) = response.next().await {
                body.extend_from_slice(&chunk.map_err(|e| classify("get_with_revision", key, e))?);
            }
            Ok(Some((revision, Bytes::from(body))))
        })
        .await
    }

    async fn head(&self, key: &str) -> Result<Option<ObjectMeta>, ObjectStoreError> {
        self.with_retries("head", key, || async {
            match self
                .control
                .get_object()
                .set_bucket(self.resource.clone())
                .set_object(key.to_string())
                .send()
                .await
            {
                Ok(object) => Ok(Some(ObjectMeta {
                    size: u64::try_from(object.size).unwrap_or(0),
                    revision: Some(Revision::GcsGeneration(object.generation)),
                })),
                Err(error) => match classify("head", key, error) {
                    ObjectStoreError::NotFound { .. } => Ok(None),
                    other => Err(other),
                },
            }
        })
        .await
    }

    async fn list_page(
        &self,
        prefix: &str,
        continuation_token: Option<String>,
        max_keys: usize,
    ) -> Result<ListPage, ObjectStoreError> {
        let page_size = i32::try_from(max_keys).unwrap_or(i32::MAX);

        self.with_retries("list_page", prefix, || async {
            let mut request = self
                .control
                .list_objects()
                .set_parent(self.resource.clone())
                .set_prefix(prefix.to_string())
                .set_page_size(page_size);
            if let Some(token) = continuation_token.clone() {
                request = request.set_page_token(token);
            }
            let response = request
                .send()
                .await
                .map_err(|e| classify("list_page", prefix, e))?;

            // Cloud Storage signals "no more pages" with an empty token rather
            // than an absent one.
            let next = Some(response.next_page_token).filter(|token| !token.is_empty());
            Ok(ListPage {
                objects: response
                    .objects
                    .into_iter()
                    .map(|object| (object.name, u64::try_from(object.size).unwrap_or(0)))
                    .collect(),
                is_truncated: next.is_some(),
                next_continuation_token: next,
            })
        })
        .await
    }

    /// Delete one object.
    ///
    /// The delete is not generation-qualified. It does not need to be: the
    /// constructor refuses buckets with object versioning or soft delete, so a
    /// key has exactly one live generation and an unqualified delete removes
    /// precisely the object the caller addressed. Deleting an absent object is
    /// not an error.
    async fn delete(&self, key: &str) -> Result<(), ObjectStoreError> {
        self.with_retries("delete", key, || async {
            match self
                .control
                .delete_object()
                .set_bucket(self.resource.clone())
                .set_object(key.to_string())
                .send()
                .await
            {
                Ok(()) => Ok(()),
                Err(error) => match classify("delete", key, error) {
                    ObjectStoreError::NotFound { .. } => Ok(()),
                    other => Err(other),
                },
            }
        })
        .await
    }

    async fn delete_objects(&self, keys: &[String]) -> Result<BulkDeleteOutcome, ObjectStoreError> {
        if keys.is_empty() {
            return Ok(BulkDeleteOutcome::default());
        }

        // Cloud Storage has no batch-delete RPC, so this is bounded-concurrency
        // individual deletes folded into the same per-key outcome the S3
        // provider reports from its batch response.
        let outcomes = futures_util::stream::iter(keys.iter().cloned())
            .map(|key| async move {
                let result = self
                    .with_retries("delete_objects", &key, || async {
                        self.control
                            .delete_object()
                            .set_bucket(self.resource.clone())
                            .set_object(key.clone())
                            .send()
                            .await
                            .map_err(|e| classify("delete_objects", &key, e))
                    })
                    .await;
                (key, result)
            })
            .buffer_unordered(BULK_DELETE_CONCURRENCY)
            .collect::<Vec<_>>()
            .await;

        let mut outcome = BulkDeleteOutcome::default();
        for (key, result) in outcomes {
            match result {
                Ok(()) => outcome.deleted += 1,
                Err(ObjectStoreError::NotFound { .. }) => outcome.already_missing += 1,
                Err(error) => {
                    outcome
                        .failed
                        .push((key, error_code(&error).to_string(), error.to_string()))
                }
            }
        }
        // `versioned_keys` stays empty by construction: a delete on a bucket
        // that passed the admission check cannot produce a version artifact.
        Ok(outcome)
    }

    async fn list_versions_page(
        &self,
        prefix: &str,
        key_marker: Option<String>,
        version_id_marker: Option<String>,
        max_keys: usize,
    ) -> Result<ObjectVersionsPage, ObjectStoreError> {
        if version_id_marker.is_some() {
            return Err(ObjectStoreError::Provider {
                operation: "list_versions_page",
                message: "GCS pagination accepts one opaque page token; a second cursor component is invalid"
                    .to_string(),
            });
        }
        let page_size = i32::try_from(max_keys).unwrap_or(i32::MAX);

        self.with_retries("list_versions_page", prefix, || async {
            let mut request = self
                .control
                .list_objects()
                .set_parent(self.resource.clone())
                .set_prefix(prefix.to_string())
                .set_versions(true)
                .set_page_size(page_size);
            if let Some(token) = key_marker.clone() {
                request = request.set_page_token(token);
            }
            let response = request
                .send()
                .await
                .map_err(|e| classify("list_versions_page", prefix, e))?;
            let next = Some(response.next_page_token).filter(|token| !token.is_empty());
            Ok(ObjectVersionsPage {
                entries: response
                    .objects
                    .into_iter()
                    .map(|object| ObjectVersionEntry {
                        key: object.name,
                        version_id: object.generation.to_string(),
                        kind: ObjectVersionKind::Object,
                        size: u64::try_from(object.size).unwrap_or(0),
                    })
                    .collect(),
                is_truncated: next.is_some(),
                next_key_marker: next,
                next_version_id_marker: None,
            })
        })
        .await
    }

    async fn delete_versions(
        &self,
        versions: &[ObjectVersionRef],
    ) -> Result<BulkDeleteOutcome, ObjectStoreError> {
        if versions.is_empty() {
            return Ok(BulkDeleteOutcome::default());
        }

        let mut parsed = Vec::with_capacity(versions.len());
        for version in versions {
            let generation = version.version_id.parse::<i64>().map_err(|_| {
                ObjectStoreError::Provider {
                    operation: "delete_versions",
                    message: format!(
                        "invalid GCS generation for object {:?}",
                        version.key
                    ),
                }
            })?;
            if generation <= 0 {
                return Err(ObjectStoreError::Provider {
                    operation: "delete_versions",
                    message: format!(
                        "non-positive GCS generation for object {:?}",
                        version.key
                    ),
                });
            }
            parsed.push((version.key.clone(), generation));
        }

        let outcomes = futures_util::stream::iter(parsed)
            .map(|(key, generation)| async move {
                let result = self
                    .with_retries("delete_versions", &key, || async {
                        self.control
                            .delete_object()
                            .set_bucket(self.resource.clone())
                            .set_object(key.clone())
                            .set_generation(generation)
                            .send()
                            .await
                            .map_err(|e| classify("delete_versions", &key, e))
                    })
                    .await;
                (key, result)
            })
            .buffer_unordered(BULK_DELETE_CONCURRENCY)
            .collect::<Vec<_>>()
            .await;

        let mut outcome = BulkDeleteOutcome::default();
        for (key, result) in outcomes {
            match result {
                Ok(()) => outcome.deleted += 1,
                Err(ObjectStoreError::NotFound { .. }) => outcome.already_missing += 1,
                Err(error) => outcome.failed.push((
                    key,
                    error_code(&error).to_string(),
                    error.to_string(),
                )),
            }
        }
        Ok(outcome)
    }

    async fn ping(&self) -> Result<(), ObjectStoreError> {
        self.list_page("", None, 1).await.map(|_| ())
    }

    /// Whether a delete on this bucket would leave a restorable copy behind.
    ///
    /// Read from bucket metadata rather than probed. Every Cloud Storage object
    /// carries a generation whether or not old ones are retained, so the S3
    /// provider's "did the response have a version id" heuristic would answer
    /// yes on a correctly configured bucket. Soft-delete retention counts too:
    /// it retains a restorable copy for exactly the same reason versioning
    /// does.
    async fn versioning_detected(&self) -> Result<bool, ObjectStoreError> {
        Ok(self
            .read_bucket_contract()
            .await?
            .retains_noncurrent_versions())
    }
}

/// Stable, bounded label for a per-key bulk-delete failure.
fn error_code(error: &ObjectStoreError) -> &'static str {
    match error {
        ObjectStoreError::NotFound { .. } => "NotFound",
        ObjectStoreError::Conflict { .. } => "PreconditionFailed",
        ObjectStoreError::Throttled { .. } => "Throttled",
        ObjectStoreError::TransportRetryable { .. } => "TransportRetryable",
        ObjectStoreError::TransportAmbiguous { .. } => "TransportAmbiguous",
        ObjectStoreError::Config(_) => "Config",
        ObjectStoreError::ObjectTooLarge { .. } => "ObjectTooLarge",
        ObjectStoreError::DigestMismatch { .. } => "DigestMismatch",
        ObjectStoreError::RevisionMismatch { .. } => "RevisionMismatch",
        ObjectStoreError::Provider { .. } => "Provider",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use google_cloud_storage::http::HeaderMap;
    use google_cloud_storage::model::bucket::{SoftDeletePolicy, Versioning};
    use google_cloud_storage::model::Bucket;

    fn http_error(status: u16) -> GcsError {
        GcsError::http(status, HeaderMap::new(), bytes::Bytes::new())
    }

    fn throttled_error(retry_after: &str) -> GcsError {
        let mut headers = HeaderMap::new();
        headers.insert("retry-after", retry_after.parse().unwrap());
        GcsError::http(429, headers, bytes::Bytes::new())
    }

    #[test]
    fn bucket_resource_uses_the_wildcard_project_form() {
        assert_eq!(
            bucket_resource("buzz-objects"),
            "projects/_/buckets/buzz-objects"
        );
    }

    /// Create-only is `ifGenerationMatch=0`; a compare-and-swap carries the
    /// observed generation verbatim.
    #[test]
    fn write_conditions_map_to_generation_preconditions() {
        assert_eq!(generation_precondition(&WriteCondition::Absent).unwrap(), 0);
        assert_eq!(
            generation_precondition(&WriteCondition::Matches(Revision::GcsGeneration(
                1_700_000_000_000_042
            )))
            .unwrap(),
            1_700_000_000_000_042
        );
    }

    /// An ETag can never predicate a generation precondition. Accepting one
    /// would have to mean dropping the precondition, which is a blind
    /// overwrite.
    #[test]
    fn a_foreign_revision_is_rejected_rather_than_downgraded() {
        let err =
            generation_precondition(&WriteCondition::Matches(Revision::S3Etag("\"abc\"".into())))
                .expect_err("an S3 ETag must never predicate a GCS generation match");
        assert!(matches!(
            err,
            ObjectStoreError::RevisionMismatch {
                expected: ProviderKind::Gcs,
                actual: ProviderKind::S3,
            }
        ));
    }

    #[test]
    fn missing_object_classifies_as_not_found() {
        let err = classify("get", "packs/x", http_error(404));
        assert!(matches!(err, ObjectStoreError::NotFound { ref key } if key == "packs/x"));
        assert!(!err.is_ambiguous());
    }

    /// A stale generation is a normal compare-and-swap conflict, never a
    /// backend failure and never a throttle.
    #[test]
    fn stale_generation_classifies_as_conflict() {
        let err = classify("put_conditional", "pointers/x", http_error(412));
        assert!(matches!(err, ObjectStoreError::Conflict { ref key } if key == "pointers/x"));
        assert!(!err.is_ambiguous() && !err.is_retryable());
    }

    #[test]
    fn throttling_classifies_as_retryable_backpressure() {
        let err = classify("put_conditional", "pointers/x", http_error(429));
        assert!(matches!(
            err,
            ObjectStoreError::Throttled {
                retry_after: None,
                ..
            }
        ));
        assert!(err.is_retryable() && !err.is_ambiguous());
    }

    #[test]
    fn retry_after_seconds_are_honoured_and_capped() {
        let err = classify("put_conditional", "pointers/x", throttled_error("3"));
        assert!(matches!(
            err,
            ObjectStoreError::Throttled {
                retry_after: Some(d),
                ..
            } if d == Duration::from_secs(3)
        ));

        let store_retry = GcsRetryConfig::default();
        let hint = Duration::from_secs(3600).min(MAX_RETRY_AFTER);
        assert_eq!(hint, MAX_RETRY_AFTER);
        assert!(store_retry.max_backoff < MAX_RETRY_AFTER);
    }

    /// An HTTP-date `Retry-After` is not misread as a duration; the caller
    /// falls back to computed backoff instead of sleeping for aeons.
    #[test]
    fn unparseable_retry_after_falls_back_to_backoff() {
        let err = classify("get", "k", throttled_error("Wed, 21 Oct 2026 07:28:00 GMT"));
        assert!(matches!(
            err,
            ObjectStoreError::Throttled {
                retry_after: None,
                ..
            }
        ));
    }

    #[test]
    fn transient_statuses_stay_classified_but_retryable() {
        for status in [408, 500, 502, 503, 504] {
            let err = classify("get", "k", http_error(status));
            assert!(
                matches!(err, ObjectStoreError::TransportRetryable { .. }),
                "status {status} should be retryable, got {err}"
            );
            assert!(err.is_retryable() && !err.is_ambiguous());
        }
    }

    /// A 403 is a real answer: permanent, classified, never dropped from the
    /// conformance probe's observer set.
    #[test]
    fn permission_denied_classifies_as_permanent_provider_failure() {
        let err = classify("put", "k", http_error(403));
        assert!(matches!(err, ObjectStoreError::Provider { .. }));
        assert!(!err.is_ambiguous() && !err.is_retryable());
    }

    /// Pre-classification failures are the only ambiguous outcomes.
    #[test]
    fn transport_failures_without_a_status_classify_as_ambiguous() {
        for error in [
            GcsError::io("connection reset by peer"),
            GcsError::connect("connection refused"),
            GcsError::timeout("deadline exceeded"),
        ] {
            let err = classify("put_conditional", "k", error);
            assert!(err.is_ambiguous(), "expected ambiguous, got {err}");
        }
    }

    /// A conditional write whose attempt was refused at connect time never
    /// reached the backend, so a later 412 is somebody else's commit.
    #[test]
    fn a_refused_connection_cannot_have_committed() {
        let refused = classify(
            "put_conditional",
            "k",
            GcsError::connect("connection refused"),
        );
        assert!(!may_have_committed(&refused));

        let reset = classify(
            "put_conditional",
            "k",
            GcsError::io("connection reset by peer"),
        );
        assert!(may_have_committed(&reset));
    }

    /// Classified answers say what happened, so they never arm the
    /// reread-and-decide path.
    #[test]
    fn classified_failures_do_not_arm_ambiguity() {
        for error in [
            classify("put_conditional", "k", http_error(429)),
            classify("put_conditional", "k", http_error(503)),
            classify("put_conditional", "k", http_error(403)),
        ] {
            assert!(!may_have_committed(&error), "{error}");
        }
    }

    fn bucket_with(versioning: Option<bool>, soft_delete_seconds: Option<u64>) -> Bucket {
        let mut bucket = Bucket::new();
        if let Some(enabled) = versioning {
            bucket.versioning = Some(Versioning::new().set_enabled(enabled));
        }
        if let Some(seconds) = soft_delete_seconds {
            let retention = google_cloud_wkt::Duration::try_from(Duration::from_secs(seconds))
                .expect("representable retention");
            bucket.soft_delete_policy =
                Some(SoftDeletePolicy::new().set_retention_duration(retention));
        }
        bucket
    }

    #[test]
    fn a_conforming_bucket_is_admitted() {
        for bucket in [
            bucket_with(None, None),
            bucket_with(Some(false), Some(0)),
            bucket_with(Some(false), None),
        ] {
            let contract = BucketContract::of(&bucket);
            assert!(!contract.retains_noncurrent_versions());
            contract.admit("buzz-objects").expect("bucket conforms");
        }
    }

    /// Versioning and soft delete are different mechanisms with the same
    /// consequence: a delete stops proving absence. Both fail construction.
    #[test]
    fn versioning_or_soft_delete_fails_admission_closed() {
        let versioned = BucketContract::of(&bucket_with(Some(true), Some(0)));
        assert!(versioned.retains_noncurrent_versions());
        let err = versioned.admit("buzz-objects").unwrap_err();
        assert!(
            matches!(err, ObjectStoreError::Config(ref m) if m.contains("object versioning is enabled")),
            "unexpected error: {err}"
        );

        let soft_deleted = BucketContract::of(&bucket_with(Some(false), Some(604_800)));
        assert!(soft_deleted.retains_noncurrent_versions());
        let err = soft_deleted.admit("buzz-objects").unwrap_err();
        assert!(
            matches!(err, ObjectStoreError::Config(ref m) if m.contains("soft-delete retention is 604800s")),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn both_violations_are_reported_together() {
        let contract = BucketContract::of(&bucket_with(Some(true), Some(604_800)));
        let err = contract.admit("buzz-objects").unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("object versioning is enabled"),
            "{message}"
        );
        assert!(message.contains("soft-delete retention"), "{message}");
    }

    #[tokio::test]
    async fn empty_bucket_and_zero_attempts_are_rejected_at_construction() {
        // Both checks run before any credential resolution or network call, so
        // they are provable without an environment.
        let mut config = GcsStoreConfig::new("");
        assert!(matches!(
            GcsObjectStore::connect(&config).await,
            Err(ObjectStoreError::Config(ref m)) if m.contains("bucket must be configured")
        ));

        config = GcsStoreConfig::new("buzz-objects");
        config.retry.max_attempts = 0;
        assert!(matches!(
            GcsObjectStore::connect(&config).await,
            Err(ObjectStoreError::Config(ref m)) if m.contains("max_attempts")
        ));
    }

    #[test]
    fn error_detail_is_bounded() {
        let payload = bytes::Bytes::from("x".repeat(4096));
        let error = GcsError::http(500, HeaderMap::new(), payload);
        assert!(detail(&error).len() <= MAX_ERROR_DETAIL + 4);
    }
}
