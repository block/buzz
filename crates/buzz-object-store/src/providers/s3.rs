//! S3 and S3-compatible provider, backed by `rust-s3`.
//!
//! This is the only module in Buzz that knows what an ETag is. Everything
//! above the seam holds a [`Revision`] and cannot tell an ETag from a Google
//! Cloud Storage generation.
//!
//! ## The 412 sharp edge
//!
//! `rust-s3`'s `fail-on-err` Cargo feature is unified ON across the build
//! graph, so non-2xx responses arrive here as `S3Error::HttpFailWithBody(code,
//! body)` *before* the caller sees `ResponseData`. A precondition failure
//! (412) is therefore an `Err` that must be reclassified as a *semantic*
//! result — [`ConditionalWrite::Conflict`] or [`ImmutableWrite::AlreadyPresent`]
//! — rather than propagated as a backend error. Empirically verified against
//! MinIO by the Git store's `probe::probe_412_surfacing`.
//!
//! ## Classified vs. unknown outcomes
//!
//! [`classify`] draws the line the Git conformance probe depends on:
//! `S3Error::{Reqwest, Http, Io}` are pre-classification failures — the racer
//! never got an answer from the backend — and map to
//! [`ObjectStoreError::TransportAmbiguous`]. Every other variant means the
//! backend *did* answer, in or out of contract, and stays a classified
//! observation. Do not widen the ambiguous set: it would let a genuine
//! conformance failure be silently dropped from the probe's observer set.

use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;

use async_trait::async_trait;
use bytes::Bytes;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use s3::creds::Credentials;
use s3::error::S3Error;
use s3::request::Request as _;
use s3::{Bucket, Region};

use crate::error::ObjectStoreError;
use crate::revision::{ConditionalWrite, ProviderKind, Revision, WriteCondition};
use crate::{
    BulkDeleteOutcome, ByteStream, ImmutableWrite, ListPage, ObjectMeta, ObjectStore,
    ObjectVersionEntry, ObjectVersionKind, ObjectVersionRef, ObjectVersionsPage,
};

/// S3 URL addressing style shared by media and Git/CAS storage.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum S3AddressingStyle {
    /// Put the bucket in the request path (`https://endpoint/bucket/key`).
    ///
    /// This preserves compatibility with the bundled MinIO deployments, whose
    /// internal DNS only resolves the endpoint hostname.
    #[default]
    Path,
    /// Put the bucket in the hostname (`https://bucket.endpoint/key`).
    ///
    /// This is the standard S3 form and is required by providers such as new
    /// Railway Storage Buckets.
    Virtual,
}

impl FromStr for S3AddressingStyle {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "path" => Ok(Self::Path),
            "virtual" => Ok(Self::Virtual),
            _ => Err(format!(
                "BUZZ_S3_ADDRESSING_STYLE must be 'path' or 'virtual', got {value:?}"
            )),
        }
    }
}

/// Connection inputs for [`S3ObjectStore::new`].
#[derive(Debug, Clone)]
pub struct S3StoreConfig {
    /// S3-compatible endpoint URL (e.g. `http://localhost:9000`).
    pub endpoint: String,
    /// Static access key, or empty to use the AWS credential chain.
    pub access_key: String,
    /// Static secret key, or empty to use the AWS credential chain.
    pub secret_key: String,
    /// Bucket name.
    pub bucket: String,
    /// Region used for SigV4 request signing.
    pub region: String,
    /// URL addressing style.
    pub addressing_style: S3AddressingStyle,
}

/// S3-compatible object storage client.
pub struct S3ObjectStore {
    bucket: Box<Bucket>,
}

impl S3ObjectStore {
    /// Build a client against an S3-compatible endpoint (e.g. MinIO).
    ///
    /// Credential selection:
    /// - If both `access_key` and `secret_key` are non-empty, use them as
    ///   static credentials (MinIO/local/dev, or any static-key deployment).
    /// - If both are empty, fall back to the AWS default credential chain via
    ///   [`Credentials::default`]: environment, shared profile, web-identity
    ///   token (IRSA on EKS — `AssumeRoleWithWebIdentity`), container, and
    ///   instance-metadata providers, in that order. This lets the relay use
    ///   the pod's IAM role without long-lived static keys.
    /// - If exactly one is empty, fail: a half-configured static deployment
    ///   must surface rather than silently fall back to the chain.
    pub fn new(config: &S3StoreConfig) -> Result<Self, ObjectStoreError> {
        let region = Region::Custom {
            region: config.region.clone(),
            endpoint: config.endpoint.clone(),
        };
        let creds = match (config.access_key.is_empty(), config.secret_key.is_empty()) {
            (false, false) => Credentials::new(
                Some(&config.access_key),
                Some(&config.secret_key),
                None,
                None,
                None,
            ),
            (true, true) => Credentials::default(),
            _ => {
                return Err(ObjectStoreError::Config(
                    "s3 access_key and secret_key must be configured together, or both empty to use the AWS credential chain"
                        .to_string(),
                ));
            }
        }
        .map_err(|e| ObjectStoreError::Config(e.to_string()))?;
        let bucket = Bucket::new(&config.bucket, region, creds)
            .map_err(|e| ObjectStoreError::Config(e.to_string()))?;
        let bucket = match config.addressing_style {
            S3AddressingStyle::Path => bucket.with_path_style(),
            S3AddressingStyle::Virtual => bucket,
        };
        Ok(Self { bucket })
    }

    /// The bucket's public URL, for diagnostics and tests.
    pub fn url(&self) -> String {
        self.bucket.url()
    }

    /// Whether the client signs and routes in path-addressing style.
    pub fn is_path_style(&self) -> bool {
        self.bucket.is_path_style()
    }

    /// The signing region configured on the client.
    pub fn region(&self) -> &Region {
        &self.bucket.region
    }

    /// Build the precondition headers for one conditional write.
    fn condition_headers(
        condition: &WriteCondition,
    ) -> Result<axum::http::HeaderMap, ObjectStoreError> {
        let mut headers = axum::http::HeaderMap::new();
        match condition {
            WriteCondition::Absent => {
                headers.insert(axum::http::header::IF_NONE_MATCH, "*".parse().unwrap());
            }
            WriteCondition::Matches(revision) => {
                let tag = revision.expect_s3_etag()?;
                headers.insert(
                    axum::http::header::IF_MATCH,
                    tag.parse().map_err(|_| ObjectStoreError::Provider {
                        operation: "put_conditional",
                        message: format!("invalid etag {tag}"),
                    })?,
                );
            }
        }
        Ok(headers)
    }

    /// Read the ETag out of a response's headers, in either casing.
    fn etag_of(headers: &std::collections::HashMap<String, String>) -> Option<String> {
        headers.get("etag").or_else(|| headers.get("ETag")).cloned()
    }
}

/// Map a `rust-s3` failure into the provider-neutral taxonomy.
///
/// See the module docs: only pre-classification failures may become
/// [`ObjectStoreError::TransportAmbiguous`].
fn classify(operation: &'static str, key: &str, error: S3Error) -> ObjectStoreError {
    let message = error.to_string();
    match error {
        S3Error::Reqwest(_) | S3Error::Http(_) | S3Error::Io(_) => {
            ObjectStoreError::TransportAmbiguous { operation, message }
        }
        S3Error::HttpFailWithBody(404, _) => ObjectStoreError::NotFound { key: key.into() },
        S3Error::HttpFailWithBody(412, _) => ObjectStoreError::Conflict { key: key.into() },
        S3Error::HttpFailWithBody(429, _) => ObjectStoreError::Throttled {
            operation,
            retry_after: None,
        },
        S3Error::HttpFailWithBody(500 | 502 | 503 | 504, _) => {
            ObjectStoreError::TransportRetryable { operation, message }
        }
        _ => ObjectStoreError::Provider { operation, message },
    }
}

/// Fold one `DeleteObjects` response into per-key outcomes.
///
/// Historical MinIO releases report already-absent keys as
/// `NoSuchKey`/`NoSuchVersion` errors instead of deleted; both map to
/// `already_missing` to keep checkpointed retry idempotent.
fn fold_bulk_delete_result(result: s3::serde_types::DeleteObjectsResult) -> BulkDeleteOutcome {
    fold_delete_result(result, DeleteMode::Unversioned)
}

fn fold_version_delete_result(result: s3::serde_types::DeleteObjectsResult) -> BulkDeleteOutcome {
    fold_delete_result(result, DeleteMode::ExplicitVersion)
}

enum DeleteMode {
    Unversioned,
    ExplicitVersion,
}

fn fold_delete_result(
    result: s3::serde_types::DeleteObjectsResult,
    mode: DeleteMode,
) -> BulkDeleteOutcome {
    let mut outcome = BulkDeleteOutcome::default();
    for deleted in result.deleted {
        let has_version_artifact = deleted.delete_marker == Some(true)
            || deleted.delete_marker_version_id.is_some()
            || deleted.version_id.is_some();
        match mode {
            DeleteMode::Unversioned if has_version_artifact => {
                outcome.versioned_keys.push(deleted.key);
            }
            DeleteMode::Unversioned | DeleteMode::ExplicitVersion => outcome.deleted += 1,
        }
    }
    for error in result.errors {
        if error.code == "NoSuchKey" || error.code == "NoSuchVersion" {
            outcome.already_missing += 1;
        } else {
            outcome.failed.push((error.key, error.code, error.message));
        }
    }
    outcome
}

#[derive(Debug, Default)]
struct ListVersionFields {
    key: Option<String>,
    version_id: Option<String>,
    size: Option<u64>,
}

fn local_name(name: &[u8]) -> &[u8] {
    name.rsplit(|byte| *byte == b':').next().unwrap_or(name)
}

fn version_xml_error(error: impl std::fmt::Display) -> ObjectStoreError {
    ObjectStoreError::Provider {
        operation: "list_versions_page",
        message: error.to_string(),
    }
}

fn read_element_text(
    reader: &mut Reader<&[u8]>,
    start: &BytesStart<'_>,
) -> Result<String, ObjectStoreError> {
    reader
        .read_text(start.to_end().name())
        .map(|text| text.into_owned())
        .map_err(version_xml_error)
}

fn skip_element(
    reader: &mut Reader<&[u8]>,
    start: &BytesStart<'_>,
) -> Result<(), ObjectStoreError> {
    reader
        .read_to_end(start.to_end().name())
        .map_err(version_xml_error)?;
    Ok(())
}

fn parse_list_version_entry(
    reader: &mut Reader<&[u8]>,
    start: &BytesStart<'_>,
    kind: ObjectVersionKind,
) -> Result<ObjectVersionEntry, ObjectStoreError> {
    let mut fields = ListVersionFields::default();
    loop {
        match reader.read_event().map_err(version_xml_error)? {
            Event::Start(child) => match local_name(child.local_name().as_ref()) {
                b"Key" => fields.key = Some(read_element_text(reader, &child)?),
                b"VersionId" => fields.version_id = Some(read_element_text(reader, &child)?),
                b"Size" => {
                    fields.size = Some(
                        read_element_text(reader, &child)?
                            .parse::<u64>()
                            .map_err(version_xml_error)?,
                    );
                }
                _ => skip_element(reader, &child)?,
            },
            Event::Empty(child) => match local_name(child.local_name().as_ref()) {
                b"Key" => fields.key = Some(String::new()),
                b"VersionId" => fields.version_id = Some(String::new()),
                b"Size" => fields.size = Some(0),
                _ => {}
            },
            Event::End(end) if end.name().as_ref() == start.to_end().name().as_ref() => {
                let key = fields
                    .key
                    .ok_or_else(|| version_xml_error("ListObjectVersions entry missing Key"))?;
                let version_id = fields.version_id.ok_or_else(|| {
                    version_xml_error("ListObjectVersions entry missing VersionId")
                })?;
                return Ok(ObjectVersionEntry {
                    key,
                    version_id,
                    kind,
                    size: if kind == ObjectVersionKind::Object {
                        fields.size.unwrap_or(0)
                    } else {
                        0
                    },
                });
            }
            Event::Eof => {
                return Err(version_xml_error(
                    "unexpected EOF inside ListObjectVersions entry",
                ));
            }
            _ => {}
        }
    }
}

fn parse_object_versions_page(xml: &[u8]) -> Result<ObjectVersionsPage, ObjectStoreError> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(true);
    let mut entries = Vec::new();
    let mut next_key_marker = None;
    let mut next_version_id_marker = None;
    let mut is_truncated = false;

    loop {
        match reader.read_event().map_err(version_xml_error)? {
            Event::Start(start) => match local_name(start.local_name().as_ref()) {
                b"Version" => entries.push(parse_list_version_entry(
                    &mut reader,
                    &start,
                    ObjectVersionKind::Object,
                )?),
                b"DeleteMarker" => entries.push(parse_list_version_entry(
                    &mut reader,
                    &start,
                    ObjectVersionKind::DeleteMarker,
                )?),
                b"IsTruncated" => {
                    is_truncated =
                        read_element_text(&mut reader, &start)?.eq_ignore_ascii_case("true");
                }
                b"NextKeyMarker" => {
                    next_key_marker = Some(read_element_text(&mut reader, &start)?);
                }
                b"NextVersionIdMarker" => {
                    next_version_id_marker = Some(read_element_text(&mut reader, &start)?);
                }
                b"ListVersionsResult" => {}
                _ => skip_element(&mut reader, &start)?,
            },
            Event::Empty(start) => match local_name(start.local_name().as_ref()) {
                b"NextKeyMarker" => next_key_marker = Some(String::new()),
                b"NextVersionIdMarker" => next_version_id_marker = Some(String::new()),
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }

    Ok(ObjectVersionsPage {
        entries,
        next_key_marker,
        next_version_id_marker,
        is_truncated,
    })
}

#[async_trait]
impl ObjectStore for S3ObjectStore {
    fn provider(&self) -> ProviderKind {
        ProviderKind::S3
    }

    async fn put(
        &self,
        key: &str,
        bytes: &[u8],
        content_type: &str,
    ) -> Result<(), ObjectStoreError> {
        self.bucket
            .put_object_with_content_type(key, bytes, content_type)
            .await
            .map_err(|e| classify("put", key, e))?;
        Ok(())
    }

    async fn put_file(
        &self,
        key: &str,
        path: &Path,
        content_type: &str,
    ) -> Result<(), ObjectStoreError> {
        /// 8 MiB read buffer — the file is streamed, never held whole in RAM.
        const BUF: usize = 8 * 1024 * 1024;

        let file = tokio::fs::File::open(path)
            .await
            .map_err(|e| ObjectStoreError::Provider {
                operation: "put_file",
                message: e.to_string(),
            })?;
        let mut reader = tokio::io::BufReader::with_capacity(BUF, file);

        self.bucket
            .put_object_stream_with_content_type(&mut reader, key, content_type)
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
        let headers = Self::condition_headers(&WriteCondition::Absent)?;
        match self
            .bucket
            .put_object_with_content_type_and_headers(key, bytes, content_type, Some(headers))
            .await
        {
            Ok(resp) if (200..300).contains(&resp.status_code()) => Ok(ImmutableWrite::Created),
            Err(S3Error::HttpFailWithBody(412, _)) => Ok(ImmutableWrite::AlreadyPresent),
            Ok(resp) => Err(ObjectStoreError::Provider {
                operation: "put_immutable",
                message: format!("unexpected status {}", resp.status_code()),
            }),
            Err(e) => Err(classify("put_immutable", key, e)),
        }
    }

    async fn put_conditional(
        &self,
        key: &str,
        bytes: &[u8],
        content_type: &str,
        condition: WriteCondition,
    ) -> Result<ConditionalWrite, ObjectStoreError> {
        let headers = Self::condition_headers(&condition)?;
        match self
            .bucket
            .put_object_with_content_type_and_headers(key, bytes, content_type, Some(headers))
            .await
        {
            Ok(resp) if (200..300).contains(&resp.status_code()) => {
                let etag = Self::etag_of(&resp.headers()).ok_or({
                    // Fail closed: a conditional write we cannot chain (because
                    // the backend returned no ETag) is not a commit — it is a
                    // non-conforming backend. The conformance probe catches
                    // this at boot; in production we would rather refuse than
                    // hand the caller an empty token and force-fail the next
                    // compare-and-swap.
                    ObjectStoreError::Provider {
                        operation: "put_conditional",
                        message: "conditional write committed but the response carried no ETag \
                                  (backend does not satisfy revision-token consistency)"
                            .to_string(),
                    }
                })?;
                Ok(ConditionalWrite::Committed(Revision::S3Etag(etag)))
            }
            Err(S3Error::HttpFailWithBody(412, _)) => Ok(ConditionalWrite::Conflict),
            Ok(resp) => Err(ObjectStoreError::Provider {
                operation: "put_conditional",
                message: format!("unexpected status {}", resp.status_code()),
            }),
            Err(e) => Err(classify("put_conditional", key, e)),
        }
    }

    async fn get(&self, key: &str) -> Result<Bytes, ObjectStoreError> {
        let resp = self
            .bucket
            .get_object(key)
            .await
            .map_err(|e| classify("get", key, e))?;
        Ok(Bytes::from(resp.to_vec()))
    }

    async fn get_range(&self, key: &str, start: u64, end: u64) -> Result<Bytes, ObjectStoreError> {
        let resp = self
            .bucket
            .get_object_range(key, start, Some(end))
            .await
            .map_err(|e| classify("get_range", key, e))?;
        Ok(Bytes::from(resp.to_vec()))
    }

    async fn get_stream(&self, key: &str) -> Result<ByteStream, ObjectStoreError> {
        let response = self
            .bucket
            .get_object_stream(key)
            .await
            .map_err(|e| classify("get_stream", key, e))?;

        if response.status_code == 404 {
            return Err(ObjectStoreError::NotFound { key: key.into() });
        }

        let operation = "get_stream";
        let stream = futures_util::StreamExt::map(response.bytes, move |chunk| {
            chunk.map_err(|e| ObjectStoreError::Provider {
                operation,
                message: e.to_string(),
            })
        });
        Ok(Box::pin(stream))
    }

    async fn get_with_revision(
        &self,
        key: &str,
    ) -> Result<Option<(Revision, Bytes)>, ObjectStoreError> {
        match self.bucket.get_object(key).await {
            Ok(resp) => {
                let etag = Self::etag_of(&resp.headers()).ok_or(ObjectStoreError::Provider {
                    operation: "get_with_revision",
                    message: "response carried no ETag".to_string(),
                })?;
                Ok(Some((Revision::S3Etag(etag), Bytes::from(resp.to_vec()))))
            }
            Err(S3Error::HttpFailWithBody(404, _)) => Ok(None),
            Err(e) => Err(classify("get_with_revision", key, e)),
        }
    }

    async fn head(&self, key: &str) -> Result<Option<ObjectMeta>, ObjectStoreError> {
        match self.bucket.head_object(key).await {
            Ok((result, _)) => Ok(Some(ObjectMeta {
                size: result.content_length.unwrap_or(0) as u64,
                revision: result.e_tag.map(Revision::S3Etag),
            })),
            Err(S3Error::HttpFailWithBody(404, _)) => Ok(None),
            Err(e) => Err(classify("head", key, e)),
        }
    }

    async fn list_page(
        &self,
        prefix: &str,
        continuation_token: Option<String>,
        max_keys: usize,
    ) -> Result<ListPage, ObjectStoreError> {
        // Wraps rust-s3's manual `list_page`, NOT the auto-paginating `list`,
        // which has no cap.
        let (result, _status) = self
            .bucket
            .list_page(
                prefix.to_string(),
                None,
                continuation_token,
                None,
                Some(max_keys),
            )
            .await
            .map_err(|e| classify("list_page", prefix, e))?;
        Ok(ListPage {
            objects: result
                .contents
                .into_iter()
                .map(|obj| (obj.key, obj.size))
                .collect(),
            next_continuation_token: result.next_continuation_token,
            is_truncated: result.is_truncated,
        })
    }

    async fn delete(&self, key: &str) -> Result<(), ObjectStoreError> {
        self.bucket
            .delete_object(key)
            .await
            .map_err(|e| classify("delete", key, e))?;
        Ok(())
    }

    async fn delete_objects(&self, keys: &[String]) -> Result<BulkDeleteOutcome, ObjectStoreError> {
        if keys.is_empty() {
            return Ok(BulkDeleteOutcome::default());
        }
        let identifiers = keys
            .iter()
            .map(|key| s3::serde_types::ObjectIdentifier::new(key.clone()))
            .collect::<Vec<_>>();
        let result = self
            .bucket
            .delete_objects(identifiers)
            .await
            .map_err(|e| classify("delete_objects", "", e))?;
        Ok(fold_bulk_delete_result(result))
    }

    async fn list_versions_page(
        &self,
        prefix: &str,
        key_marker: Option<String>,
        version_id_marker: Option<String>,
        max_keys: usize,
    ) -> Result<ObjectVersionsPage, ObjectStoreError> {
        let mut query = HashMap::from([
            ("versions".to_string(), String::new()),
            ("prefix".to_string(), prefix.to_string()),
            ("max-keys".to_string(), max_keys.to_string()),
        ]);
        if let Some(marker) = key_marker {
            query.insert("key-marker".to_string(), marker);
        }
        if let Some(marker) = version_id_marker {
            query.insert("version-id-marker".to_string(), marker);
        }
        let bucket = self
            .bucket
            .with_extra_query(query)
            .map_err(|e| classify("list_versions_page", prefix, e))?;
        let request = s3::request::tokio_backend::ReqwestRequest::new(
            &bucket,
            "/",
            s3::command::Command::GetObject,
        )
        .await
        .map_err(|e| classify("list_versions_page", prefix, e))?;
        let response = request
            .response_data(false)
            .await
            .map_err(|e| classify("list_versions_page", prefix, e))?;
        if response.status_code() >= 300 {
            return Err(ObjectStoreError::Provider {
                operation: "list_versions_page",
                message: format!(
                    "unexpected status {}: {}",
                    response.status_code(),
                    response.as_str().unwrap_or("")
                ),
            });
        }
        parse_object_versions_page(response.as_slice())
    }

    async fn delete_versions(
        &self,
        versions: &[ObjectVersionRef],
    ) -> Result<BulkDeleteOutcome, ObjectStoreError> {
        if versions.is_empty() {
            return Ok(BulkDeleteOutcome::default());
        }
        let identifiers = versions
            .iter()
            .map(|version| {
                s3::serde_types::ObjectIdentifier::with_version(
                    version.key.clone(),
                    version.version_id.clone(),
                )
            })
            .collect::<Vec<_>>();
        let result = self
            .bucket
            .delete_objects(identifiers)
            .await
            .map_err(|e| classify("delete_versions", "", e))?;
        Ok(fold_version_delete_result(result))
    }

    async fn ping(&self) -> Result<(), ObjectStoreError> {
        self.list_page("", None, 1).await.map(|_| ())
    }

    /// Detect whether the bucket has ever had versioning enabled.
    ///
    /// `rust-s3` exposes no `GetBucketVersioning`, so this writes and inspects
    /// a short-lived probe object instead: versioning-enabled (and
    /// versioning-suspended) buckets stamp new writes with a version id.
    ///
    /// This heuristic is S3-specific by construction and does not translate to
    /// providers where every object carries a version token; those must
    /// implement the check against real bucket metadata.
    async fn versioning_detected(&self) -> Result<bool, ObjectStoreError> {
        let key = format!("probe/deletion-versioning-{}", uuid::Uuid::new_v4());
        self.put(&key, b"buzz deletion versioning probe", "text/plain")
            .await?;
        let inspected = self.bucket.head_object(&key).await;
        let removed = self.bucket.delete_object(&key).await;
        let (head, _) = inspected.map_err(|e| classify("versioning_detected", &key, e))?;
        removed.map_err(|e| classify("versioning_detected", &key, e))?;
        Ok(head.version_id.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(access: &str, secret: &str) -> S3StoreConfig {
        S3StoreConfig {
            endpoint: "http://localhost:9000".to_string(),
            access_key: access.to_string(),
            secret_key: secret.to_string(),
            bucket: "buzz-media".to_string(),
            region: "us-west-2".to_string(),
            addressing_style: S3AddressingStyle::Path,
        }
    }

    /// Static keys present: builds a client without touching the AWS
    /// credential chain (no env/metadata access), and the signing region
    /// comes from config rather than a hardcoded "us-east-1".
    #[test]
    fn static_keys_build_client_with_configured_region() {
        let store =
            S3ObjectStore::new(&config("buzz_dev", "buzz_dev_secret")).expect("static creds");
        match store.region() {
            Region::Custom { region, .. } => assert_eq!(region, "us-west-2"),
            other => panic!("expected Custom region, got {other:?}"),
        }
        assert_eq!(store.provider(), ProviderKind::S3);
    }

    #[test]
    fn constructor_applies_both_addressing_styles() {
        let path =
            S3ObjectStore::new(&config("buzz_dev", "buzz_dev_secret")).expect("path-style client");
        assert!(path.is_path_style());
        assert_eq!(path.url(), "http://localhost:9000/buzz-media");

        let mut virtual_config = config("buzz_dev", "buzz_dev_secret");
        virtual_config.addressing_style = S3AddressingStyle::Virtual;
        let virtual_hosted = S3ObjectStore::new(&virtual_config).expect("virtual-hosted client");
        assert!(!virtual_hosted.is_path_style());
        assert_eq!(virtual_hosted.url(), "http://buzz-media.localhost:9000");
    }

    #[test]
    fn partial_static_keys_are_rejected() {
        for (access, secret) in [("buzz_dev", ""), ("", "buzz_dev_secret")] {
            let err = match S3ObjectStore::new(&config(access, secret)) {
                Ok(_) => panic!("partial static creds must not silently use credential chain"),
                Err(err) => err,
            };
            assert!(
                matches!(err, ObjectStoreError::Config(ref msg) if msg.contains("must be configured together")),
                "unexpected error: {err}"
            );
        }
    }

    /// Pre-classification failures are the *only* ambiguous outcomes. The Git
    /// conformance probe drops exactly this set from its observer count, so
    /// widening it would let a real conformance failure vanish.
    #[test]
    fn transport_failures_classify_as_ambiguous() {
        let io = classify("put", "k", S3Error::Io(std::io::Error::other("reset")));
        assert!(io.is_ambiguous(), "io error must be ambiguous: {io}");
    }

    #[test]
    fn precondition_failure_classifies_as_conflict() {
        let err = classify(
            "put_conditional",
            "pointers/x",
            S3Error::HttpFailWithBody(412, "PreconditionFailed".into()),
        );
        assert!(!err.is_ambiguous());
        assert!(matches!(err, ObjectStoreError::Conflict { ref key } if key == "pointers/x"));
    }

    #[test]
    fn missing_object_classifies_as_not_found() {
        let err = classify(
            "get",
            "packs/x",
            S3Error::HttpFailWithBody(404, "NoSuchKey".into()),
        );
        assert!(matches!(err, ObjectStoreError::NotFound { ref key } if key == "packs/x"));
    }

    #[test]
    fn throttling_and_transient_statuses_stay_classified_but_retryable() {
        let throttled = classify(
            "put",
            "k",
            S3Error::HttpFailWithBody(429, "SlowDown".into()),
        );
        assert!(matches!(throttled, ObjectStoreError::Throttled { .. }));
        assert!(throttled.is_retryable() && !throttled.is_ambiguous());

        let transient = classify(
            "get",
            "k",
            S3Error::HttpFailWithBody(503, "SlowDown".into()),
        );
        assert!(matches!(
            transient,
            ObjectStoreError::TransportRetryable { .. }
        ));
        assert!(transient.is_retryable() && !transient.is_ambiguous());
    }

    /// A 403 is a real backend answer: permanent, classified, never dropped.
    #[test]
    fn permission_denied_classifies_as_permanent_provider_failure() {
        let err = classify(
            "put",
            "k",
            S3Error::HttpFailWithBody(403, "AccessDenied".into()),
        );
        assert!(matches!(err, ObjectStoreError::Provider { .. }));
        assert!(!err.is_ambiguous() && !err.is_retryable());
    }

    #[test]
    fn conditional_write_rejects_a_foreign_provider_revision() {
        let err =
            S3ObjectStore::condition_headers(&WriteCondition::Matches(Revision::GcsGeneration(7)))
                .expect_err("a GCS generation must never predicate an S3 If-Match");
        assert!(matches!(
            err,
            ObjectStoreError::RevisionMismatch {
                expected: ProviderKind::S3,
                actual: ProviderKind::Gcs,
            }
        ));
    }

    #[test]
    fn conditional_write_accepts_an_s3_revision() {
        let headers = S3ObjectStore::condition_headers(&WriteCondition::Matches(Revision::S3Etag(
            "\"abc\"".into(),
        )))
        .expect("s3 etag predicates If-Match");
        assert_eq!(
            headers.get(axum::http::header::IF_MATCH).unwrap(),
            "\"abc\""
        );
    }

    #[test]
    fn create_only_write_uses_if_none_match_star() {
        let headers = S3ObjectStore::condition_headers(&WriteCondition::Absent).expect("headers");
        assert_eq!(headers.get(axum::http::header::IF_NONE_MATCH).unwrap(), "*");
    }

    /// The bulk-delete fold is the retry-idempotence contract: legacy MinIO
    /// absent-key errors count as success, version artifacts are surfaced for
    /// fail-closed handling, and anything else stays a per-key failure.
    #[test]
    fn bulk_delete_fold_maps_absent_keys_and_version_artifacts() {
        use s3::serde_types::{DeleteError, DeleteObjectsResult, DeletedObject};
        let deleted_object = |key: &str, marker: bool| DeletedObject {
            key: key.to_string(),
            version_id: None,
            delete_marker: marker.then_some(true),
            delete_marker_version_id: marker.then(|| "v1".to_string()),
        };
        let delete_error = |key: &str, code: &str, message: &str| DeleteError {
            key: key.to_string(),
            code: code.to_string(),
            message: message.to_string(),
            version_id: None,
        };
        let result = DeleteObjectsResult {
            deleted: vec![
                deleted_object("plain", false),
                deleted_object("marked", true),
            ],
            errors: vec![
                delete_error("gone", "NoSuchKey", "absent"),
                delete_error("gone-version", "NoSuchVersion", "absent"),
                delete_error("denied", "AccessDenied", "nope"),
            ],
        };
        let outcome = fold_bulk_delete_result(result);
        assert_eq!(outcome.deleted, 1);
        assert_eq!(outcome.already_missing, 2);
        assert_eq!(outcome.versioned_keys, vec!["marked".to_string()]);
        assert_eq!(
            outcome.failed,
            vec![(
                "denied".to_string(),
                "AccessDenied".to_string(),
                "nope".to_string()
            )]
        );
    }

    #[test]
    fn explicit_version_delete_counts_version_artifacts_as_deleted() {
        use s3::serde_types::{DeleteError, DeleteObjectsResult, DeletedObject};
        let result = DeleteObjectsResult {
            deleted: vec![DeletedObject {
                key: "versioned".to_string(),
                version_id: Some("v1".to_string()),
                delete_marker: Some(true),
                delete_marker_version_id: Some("v1".to_string()),
            }],
            errors: vec![DeleteError {
                key: "already-gone".to_string(),
                code: "NoSuchVersion".to_string(),
                message: "absent".to_string(),
                version_id: Some("v0".to_string()),
            }],
        };

        let outcome = fold_version_delete_result(result);
        assert_eq!(outcome.deleted, 1);
        assert_eq!(outcome.already_missing, 1);
        assert!(outcome.versioned_keys.is_empty());
        assert!(outcome.failed.is_empty());
    }

    #[test]
    fn version_listing_preserves_objects_markers_and_dual_cursor() {
        let page = parse_object_versions_page(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<ListVersionsResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
  <IsTruncated>true</IsTruncated>
  <NextKeyMarker>_meta/tenant/a.json</NextKeyMarker>
  <NextVersionIdMarker>v-new</NextVersionIdMarker>
  <DeleteMarker>
    <Key>_meta/tenant/a.json</Key><VersionId>v-delete</VersionId>
  </DeleteMarker>
  <Version>
    <Key>_meta/tenant/a.json</Key><VersionId>v-new</VersionId><Size>42</Size>
  </Version>
</ListVersionsResult>"#,
        )
        .expect("parse S3 version page");

        assert!(page.is_truncated);
        assert_eq!(page.next_key_marker.as_deref(), Some("_meta/tenant/a.json"));
        assert_eq!(page.next_version_id_marker.as_deref(), Some("v-new"));
        assert_eq!(
            page.entries,
            vec![
                ObjectVersionEntry {
                    key: "_meta/tenant/a.json".to_string(),
                    version_id: "v-delete".to_string(),
                    kind: ObjectVersionKind::DeleteMarker,
                    size: 0,
                },
                ObjectVersionEntry {
                    key: "_meta/tenant/a.json".to_string(),
                    version_id: "v-new".to_string(),
                    kind: ObjectVersionKind::Object,
                    size: 42,
                },
            ]
        );
    }
}
