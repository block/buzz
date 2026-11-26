//! The OpenSearch message index: layout, writes and searches.
//!
//! One alias, `{prefix}messages`, points at the index of the mapping version
//! this code writes, `{prefix}messages-v1`. A document is one message, with
//! id `community:event_id`. Documents are routed by id, so messages spread
//! evenly over shards, and every search filters on the community first: that
//! filter, not routing, is the multi-tenant fence.
//!
//! Message text is indexed but not kept in the stored document source. The
//! relay re-reads every hit from Postgres, so the index holds only what search
//! needs.

use std::ops::Range;
use std::time::Duration;

use aws_config::Region;
use aws_credential_types::provider::SharedCredentialsProvider;
use buzz_core::CommunityId;
use buzz_db::search_outbox::SearchDocument;
use opensearch::auth::Credentials;
use opensearch::http::response::Response;
use opensearch::http::transport::{SingleNodeConnectionPool, TransportBuilder};
use opensearch::http::Url;
use opensearch::indices::IndicesRefreshParts;
use opensearch::indices::{
    IndicesCreateParts, IndicesDeleteParts, IndicesExistsParts, IndicesGetSettingsParts,
};
use opensearch::params::VersionType;
use opensearch::{BulkOperation, BulkParts, OpenSearch, SearchParts};
use serde_json::{json, Value};

use crate::aws_credentials::{default_chain, CachedCredentials};
use crate::query::{normalized_search_text, page_bounds, PageBounds};
use crate::{
    ChannelScope, SearchError, SearchHit, SearchMode, SearchQuery, SearchResult, SearchSort,
};

/// Mapping version this code writes and searches. Bump it, with a reindex,
/// whenever the mapping or analysis changes.
const MAPPING_VERSION: u32 = 1;

/// Index names used when the configuration sets no prefix.
pub const DEFAULT_INDEX_PREFIX: &str = "buzz-";

/// Bound on every request, so a stalled cluster cannot hold an indexer batch
/// (and its admitted transaction) or a search open.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Bound on a bulk write, which is slower than a search: the first writes
/// into a new Serverless index took 20 seconds. A write that times out is
/// sent again while the first may still be applied late.
const BULK_TIMEOUT: Duration = Duration::from_secs(60);

/// The status OpenSearch throttles with, notably Serverless while it scales.
const THROTTLED: u16 = 429;
/// Item status when the index holds the document at a newer version.
const VERSION_CONFLICT: u64 = 409;

/// Most bytes of documents sent in one bulk request. A batch over it goes
/// out as several requests: OpenSearch refuses a request above its
/// `http.max_content_length` (10 MiB on the smallest AWS domains) with HTTP
/// 413, and a refused request would keep its community's queue from moving.
const MAX_BULK_BYTES: usize = 8 * 1024 * 1024;
/// Bytes allowed for one item's action line, and for a whole delete.
const BULK_ITEM_OVERHEAD: usize = 256;

/// What a bulk write did, item by item.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BulkOutcome {
    /// Events whose item failed: their outbox rows stay queued.
    pub failed: Vec<[u8; 32]>,
    /// How many of those OpenSearch refused for good (HTTP 400 or 413):
    /// retrying them fails the same way until the document or mapping
    /// changes.
    pub rejected: usize,
    /// How many of those the index already held at this version or a newer
    /// one (HTTP 409). A request applied after its batch ended has no one
    /// reading its response, and a live batch holds its events' partitions,
    /// so it is their newest writer: its conflict means the version source
    /// went back or repeated, and its newer state must stay queued.
    pub conflicts: usize,
    /// Whether OpenSearch throttled the request or any item (HTTP 429).
    pub throttled: bool,
    /// The first failure's error type: a fixed description, never content.
    pub reason: Option<String>,
}

/// OpenSearch's default `index.max_result_window`: `from + size` beyond it is
/// rejected, so pages past it are empty.
const MAX_RESULT_WINDOW: i64 = 10_000;

/// How to reach an OpenSearch cluster.
#[derive(Clone)]
pub struct OpenSearchConfig {
    /// Cluster address, for example `http://localhost:9200`.
    pub url: String,
    /// HTTP basic credentials, if the cluster needs them.
    pub basic_auth: Option<(String, String)>,
    /// Sign requests with AWS SigV4, if the cluster needs it. Not combined
    /// with basic credentials.
    pub aws_sigv4: Option<AwsSigV4>,
    /// Prefix of the alias and index names. The relay always uses
    /// [`DEFAULT_INDEX_PREFIX`]; tests set their own to share a cluster.
    pub index_prefix: String,
}

impl std::fmt::Debug for OpenSearchConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenSearchConfig")
            .field("url", &self.url)
            .field(
                "basic_auth",
                &self.basic_auth.as_ref().map(|(user, _)| user),
            )
            .field("aws_sigv4", &self.aws_sigv4)
            .field("index_prefix", &self.index_prefix)
            .finish()
    }
}

/// AWS SigV4 request signing, for Amazon OpenSearch Service. Credentials come
/// from the AWS default chain (environment, shared profile, web identity
/// token, container role, instance role) and are reloaded before they expire.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AwsSigV4 {
    /// Signing service name: `es` for a managed domain, `aoss` for a
    /// Serverless collection.
    pub service: String,
    /// AWS region of the cluster, for example `us-east-1`.
    pub region: String,
}

/// The message index on one OpenSearch cluster.
#[derive(Clone)]
pub struct SearchIndex {
    client: OpenSearch,
    alias: String,
    index: String,
}

impl std::fmt::Debug for SearchIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchIndex")
            .field("alias", &self.alias)
            .field("index", &self.index)
            .finish_non_exhaustive()
    }
}

impl SearchIndex {
    /// A client for the index described by `config`. Makes no request.
    pub fn new(config: &OpenSearchConfig) -> Result<Self, SearchError> {
        let url = Url::parse(&config.url)
            .map_err(|_| SearchError::Engine("invalid OpenSearch URL".into()))?;
        let mut transport =
            TransportBuilder::new(SingleNodeConnectionPool::new(url)).timeout(REQUEST_TIMEOUT);
        if let Some((user, password)) = &config.basic_auth {
            transport = transport.auth(Credentials::Basic(user.clone(), password.clone()));
        }
        if let Some(sigv4) = &config.aws_sigv4 {
            if config.basic_auth.is_some() {
                return Err(SearchError::Engine(
                    "basic auth and AWS SigV4 cannot be combined".into(),
                ));
            }
            let region = Region::new(sigv4.region.clone());
            let credentials = CachedCredentials::new(default_chain(region.clone()));
            transport = transport
                .auth(Credentials::AwsSigV4(
                    SharedCredentialsProvider::new(credentials),
                    region,
                ))
                .service_name(&sigv4.service);
        }
        let transport = transport
            .build()
            .map_err(|_| SearchError::Engine("OpenSearch client setup failed".into()))?;
        Ok(Self {
            client: OpenSearch::new(transport),
            alias: format!("{}messages", config.index_prefix),
            index: format!("{}messages-v{MAPPING_VERSION}", config.index_prefix),
        })
    }

    /// Create the versioned index behind the alias unless the alias exists.
    /// Returns whether this call created it. For tests and tools: the
    /// indexer clears the backfill progress between `missing` and `create`.
    pub async fn ensure(&self) -> Result<bool, SearchError> {
        if !self.missing().await? {
            return Ok(false);
        }
        self.create().await
    }

    /// Whether the cluster answers that neither the alias nor the versioned
    /// index exists. Any other answer is an error, so a cluster that fails
    /// or throttles a lookup is never taken for one that lost the index.
    pub(crate) async fn missing(&self) -> Result<bool, SearchError> {
        if self.exists(&self.alias).await? {
            return Ok(false);
        }
        if self.exists(&self.index).await? {
            return Err(SearchError::Engine("index exists without its alias".into()));
        }
        Ok(true)
    }

    /// Create the versioned index with its alias. Returns whether this call
    /// created it: false when another relay created it first.
    pub(crate) async fn create(&self) -> Result<bool, SearchError> {
        let response = self
            .client
            .indices()
            .create(IndicesCreateParts::Index(&self.index))
            .body(index_definition(&self.alias))
            .send()
            .await
            .map_err(transport_error)?;
        // Another relay may have created it, with its alias, first.
        if response.status_code().as_u16() == 400 {
            let body: Value = response.json().await.map_err(transport_error)?;
            if body["error"]["type"] == "resource_already_exists_exception" {
                if self.exists(&self.alias).await? {
                    return Ok(false);
                }
                return Err(SearchError::Engine("index exists without its alias".into()));
            }
            return Err(SearchError::Engine("index creation rejected".into()));
        }
        checked(response).await.map(|_| true)
    }

    /// Whether `name`, an index or alias, exists: true on 200, false on 404,
    /// an error on any other status.
    async fn exists(&self, name: &str) -> Result<bool, SearchError> {
        let response = self
            .client
            .indices()
            .exists(IndicesExistsParts::Index(&[name]))
            .send()
            .await
            .map_err(transport_error)?;
        match response.status_code().as_u16() {
            200 => Ok(true),
            404 => Ok(false),
            status => Err(SearchError::Engine(format!("HTTP {status}"))),
        }
    }

    /// How long ago the versioned index was created, by the cluster's
    /// creation time and this host's clock; zero if that time is ahead.
    pub(crate) async fn age(&self) -> Result<Duration, SearchError> {
        let response = self
            .client
            .indices()
            .get_settings(IndicesGetSettingsParts::IndexName(
                &[&self.index],
                &["index.creation_date"],
            ))
            .send()
            .await
            .map_err(transport_error)?;
        let body = checked(response).await?;
        let created = body[self.index.as_str()]["settings"]["index"]["creation_date"]
            .as_str()
            .and_then(|millis| millis.parse().ok())
            .ok_or_else(|| SearchError::Engine("index creation date missing".into()))?;
        let created = std::time::UNIX_EPOCH + Duration::from_millis(created);
        Ok(std::time::SystemTime::now()
            .duration_since(created)
            .unwrap_or_default())
    }

    /// Delete the versioned index. For tests and tools.
    pub async fn delete(&self) -> Result<(), SearchError> {
        let response = self
            .client
            .indices()
            .delete(IndicesDeleteParts::Index(&[&self.index]))
            .send()
            .await
            .map_err(transport_error)?;
        if response.status_code().as_u16() == 404 {
            return Ok(());
        }
        checked(response).await.map(drop)
    }

    /// Make every write so far visible to search, instead of waiting for the
    /// refresh interval. For tests and tools.
    pub async fn refresh(&self) -> Result<(), SearchError> {
        let response = self
            .client
            .indices()
            .refresh(IndicesRefreshParts::Index(&[&self.alias]))
            .send()
            .await
            .map_err(transport_error)?;
        checked(response).await.map(drop)
    }

    /// Upsert `documents` and delete `removed` for one community through the
    /// alias, and report each item's result. A batch of more than
    /// [`MAX_BULK_BYTES`] goes out as several bulk requests, one after the
    /// other; once one is throttled or fails the rest are not sent and count
    /// as failed. After the first, a failed request's items count as failed
    /// and a refused one's as rejected, so the items of the requests already
    /// applied still count as written.
    /// Deleting a document that is not there succeeds. The request requires
    /// the alias, so a missing index fails the write instead of being created
    /// with a dynamic mapping.
    ///
    /// Every item carries `version` as an external version, so the cluster
    /// keeps the newest state of each document: a request it applies after
    /// its writer gave up cannot undo a later write. An item the index
    /// already holds at this version or a newer one fails as a conflict.
    ///
    /// A throttled request (HTTP 429) comes back as an outcome in which every
    /// item failed; any other failed first request is an error.
    pub async fn write(
        &self,
        community: CommunityId,
        documents: &[SearchDocument],
        removed: &[[u8; 32]],
        version: i64,
    ) -> Result<BulkOutcome, SearchError> {
        let sizes = documents
            .iter()
            .map(|document| document_bytes(community, document))
            .chain(removed.iter().map(|_| BULK_ITEM_OVERHEAD));
        let mut outcome = BulkOutcome::default();
        let mut stopped = false;
        for range in request_ranges(sizes, MAX_BULK_BYTES) {
            let (documents, removed) = split_items(documents, removed, &range);
            if outcome.throttled || stopped {
                // Not sent: the cluster asked for less, or is failing.
                outcome
                    .failed
                    .extend(documents.iter().map(|document| document.event_id));
                outcome.failed.extend(removed);
                continue;
            }
            let part = match self
                .write_request(community, documents, removed, version)
                .await
            {
                Ok(part) => part,
                // Earlier requests were applied: report this one's items as
                // failed, so the rows of the applied ones are completed.
                Err(error) if range.start > 0 => {
                    let refused = matches!(error, SearchError::Rejected(_));
                    stopped = !refused;
                    let failed: Vec<[u8; 32]> = documents
                        .iter()
                        .map(|document| document.event_id)
                        .chain(removed.iter().copied())
                        .collect();
                    BulkOutcome {
                        rejected: if refused { failed.len() } else { 0 },
                        failed,
                        reason: Some(error.kind()),
                        ..BulkOutcome::default()
                    }
                }
                Err(error) => return Err(error),
            };
            outcome.failed.extend(part.failed);
            outcome.rejected += part.rejected;
            outcome.conflicts += part.conflicts;
            outcome.throttled = part.throttled;
            if outcome.reason.is_none() {
                outcome.reason = part.reason;
            }
        }
        Ok(outcome)
    }

    /// One bulk request of [`SearchIndex::write`].
    async fn write_request(
        &self,
        community: CommunityId,
        documents: &[SearchDocument],
        removed: &[[u8; 32]],
        version: i64,
    ) -> Result<BulkOutcome, SearchError> {
        if documents.is_empty() && removed.is_empty() {
            return Ok(BulkOutcome::default());
        }
        let index = &self.alias;
        let mut operations: Vec<BulkOperation<Value>> =
            Vec::with_capacity(documents.len() + removed.len());
        for document in documents {
            operations.push(
                BulkOperation::index(document_source(community, document))
                    .id(document_id(community, &document.event_id))
                    .index(index)
                    .version(version)
                    .version_type(VersionType::External)
                    .into(),
            );
        }
        for event_id in removed {
            operations.push(
                BulkOperation::delete(document_id(community, event_id))
                    .index(index)
                    .version(version)
                    .version_type(VersionType::External)
                    .into(),
            );
        }
        // Items come back in request order.
        let events: Vec<[u8; 32]> = documents
            .iter()
            .map(|document| document.event_id)
            .chain(removed.iter().copied())
            .collect();
        let response = self
            .client
            .bulk(BulkParts::None)
            .require_alias(true)
            .request_timeout(BULK_TIMEOUT)
            .body(operations)
            .send()
            .await
            .map_err(transport_error)?;
        if response.status_code().as_u16() == THROTTLED {
            return Ok(BulkOutcome {
                failed: events,
                rejected: 0,
                conflicts: 0,
                throttled: true,
                reason: Some(format!("HTTP {THROTTLED}")),
            });
        }
        let body = checked(response).await?;
        if body["errors"] != true {
            return Ok(BulkOutcome::default());
        }
        let items = body["items"].as_array().map(Vec::as_slice).unwrap_or(&[]);
        if items.len() != events.len() {
            return Err(SearchError::Engine(
                "bulk response items do not match the request".into(),
            ));
        }
        let mut outcome = BulkOutcome::default();
        for (event, item) in events.into_iter().zip(items) {
            let Some(result) = item.as_object().and_then(|item| item.values().next()) else {
                return Err(SearchError::Engine("malformed bulk response item".into()));
            };
            if result.get("error").is_none() {
                continue;
            }
            let status = result["status"].as_u64().unwrap_or(0);
            if status == VERSION_CONFLICT {
                outcome.conflicts += 1;
            }
            outcome.throttled |= status == u64::from(THROTTLED);
            if rejects(status) {
                outcome.rejected += 1;
            }
            if outcome.reason.is_none() {
                outcome.reason = Some(
                    result["error"]["type"]
                        .as_str()
                        .unwrap_or("unknown")
                        .to_string(),
                );
            }
            outcome.failed.push(event);
        }
        Ok(outcome)
    }

    /// Run a message search against the alias.
    pub(crate) async fn search(&self, query: &SearchQuery) -> Result<SearchResult, SearchError> {
        let PageBounds {
            page,
            per_page,
            offset,
        } = page_bounds(query);
        let Some(text) = normalized_search_text(&query.q) else {
            return Ok(SearchResult {
                hits: Vec::new(),
                page,
            });
        };
        if offset + i64::from(per_page) > MAX_RESULT_WINDOW {
            return Ok(SearchResult {
                hits: Vec::new(),
                page,
            });
        }
        let response = self
            .client
            .search(SearchParts::Index(&[&self.alias]))
            .body(search_body(query, &text, offset, per_page))
            .send()
            .await
            .map_err(transport_error)?;
        let body = checked(response).await?;
        // A partly failed search is an error, not fewer hits.
        if body["timed_out"] == true {
            return Err(SearchError::Engine("search timed out".into()));
        }
        if body["_shards"]["failed"].as_u64().unwrap_or(0) > 0 {
            return Err(SearchError::Engine("search shards failed".into()));
        }
        let hits = body["hits"]["hits"]
            .as_array()
            .into_iter()
            .flatten()
            .map(parse_hit)
            .collect::<Result<_, _>>()?;
        Ok(SearchResult { hits, page })
    }
}

/// Settings, mapping and alias of a new index.
fn index_definition(alias: &str) -> Value {
    json!({
        "settings": { "index": { "refresh_interval": "30s" } },
        "mappings": {
            "dynamic": "strict",
            "_source": { "excludes": ["content"] },
            "properties": {
                "community": { "type": "keyword" },
                "event_id": { "type": "keyword" },
                "kind": { "type": "integer" },
                "author": { "type": "keyword" },
                "channel": { "type": "keyword" },
                "channel_type": { "type": "keyword" },
                "author_is_agent": { "type": "boolean" },
                "thread_root": { "type": "keyword" },
                "mentions": { "type": "keyword" },
                "created_at": { "type": "date", "format": "epoch_second" },
                "content": { "type": "text" }
            }
        },
        "aliases": { alias: {} }
    })
}

fn document_id(community: CommunityId, event_id: &[u8; 32]) -> String {
    format!("{}:{}", community.as_uuid(), hex::encode(event_id))
}

/// The bytes `document` takes in a bulk request.
fn document_bytes(community: CommunityId, document: &SearchDocument) -> usize {
    BULK_ITEM_OVERHEAD
        + serde_json::to_vec(&document_source(community, document)).map_or(0, |source| source.len())
}

/// Split items of the given sizes, in order, into runs of at most `limit`
/// bytes each. An item over the limit gets a run of its own.
fn request_ranges(sizes: impl Iterator<Item = usize>, limit: usize) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let (mut start, mut end, mut bytes) = (0, 0, 0usize);
    for size in sizes {
        if end > start && bytes + size > limit {
            ranges.push(start..end);
            (start, bytes) = (end, 0);
        }
        bytes += size;
        end += 1;
    }
    if end > start {
        ranges.push(start..end);
    }
    ranges
}

/// The part of a batch, counted documents first, that `range` covers.
fn split_items<'a>(
    documents: &'a [SearchDocument],
    removed: &'a [[u8; 32]],
    range: &Range<usize>,
) -> (&'a [SearchDocument], &'a [[u8; 32]]) {
    let count = documents.len();
    (
        &documents[range.start.min(count)..range.end.min(count)],
        &removed[range.start.saturating_sub(count)..range.end.saturating_sub(count)],
    )
}

fn document_source(community: CommunityId, document: &SearchDocument) -> Value {
    json!({
        "community": community.as_uuid().to_string(),
        "event_id": hex::encode(document.event_id),
        "kind": document.kind,
        "author": hex::encode(document.pubkey),
        "channel": document.channel_id.map(|id| id.to_string()),
        "channel_type": document.channel_type,
        "author_is_agent": document.author_is_agent,
        "thread_root": document.thread_root.map(hex::encode),
        "mentions": document.mentions,
        "created_at": document.created_at.timestamp(),
        "content": document.content,
    })
}

/// The request body of a search: filters on the community first, then on
/// scope, kinds, authors and time; the text must match every word.
fn search_body(query: &SearchQuery, text: &str, offset: i64, per_page: u32) -> Value {
    let mut filter =
        vec![json!({ "term": { "community": query.community.as_uuid().to_string() } })];
    let channels = |ids: &[uuid::Uuid]| json!({ "terms": { "channel": ids.iter().map(|id| id.to_string()).collect::<Vec<_>>() } });
    let channel_less = json!({ "bool": { "must_not": { "exists": { "field": "channel" } } } });
    match &query.channel_scope {
        ChannelScope::Any => {}
        ChannelScope::ChannelLessOnly => filter.push(channel_less),
        ChannelScope::Channels(ids) => filter.push(channels(ids)),
        ChannelScope::ChannelsOrChannelLess(ids) => filter.push(json!({
            "bool": { "should": [channels(ids), channel_less], "minimum_should_match": 1 }
        })),
    }
    if let Some(kinds) = query.kinds.as_ref().filter(|kinds| !kinds.is_empty()) {
        filter.push(json!({ "terms": { "kind": kinds } }));
    }
    if let Some(authors) = query.authors.as_ref().filter(|authors| !authors.is_empty()) {
        let authors: Vec<String> = authors.iter().map(hex::encode).collect();
        filter.push(json!({ "terms": { "author": authors } }));
    }
    let mut range = serde_json::Map::new();
    if let Some(since) = query.since {
        range.insert("gte".into(), since.into());
    }
    if let Some(until) = query.until {
        range.insert("lte".into(), until.into());
    }
    if !range.is_empty() {
        filter.push(json!({ "range": { "created_at": range } }));
    }
    let text_match = match query.mode {
        SearchMode::FullText => {
            json!({ "match": { "content": { "query": text, "operator": "and" } } })
        }
        SearchMode::Prefix => {
            json!({ "match_bool_prefix": { "content": { "query": text, "operator": "and" } } })
        }
    };
    let tiebreak = [
        json!({ "created_at": "desc" }),
        json!({ "event_id": "asc" }),
    ];
    let sort: Vec<Value> = match query.sort {
        SearchSort::Relevance => std::iter::once(json!("_score")).chain(tiebreak).collect(),
        SearchSort::Newest => tiebreak.into(),
    };
    json!({
        "from": offset,
        "size": per_page,
        "track_total_hits": false,
        "track_scores": true,
        "_source": ["event_id", "kind", "author", "channel", "created_at"],
        "query": { "bool": { "filter": filter, "must": [text_match] } },
        "sort": sort
    })
}

fn parse_hit(hit: &Value) -> Result<SearchHit, SearchError> {
    let invalid = || SearchError::Engine("unexpected search hit shape".into());
    let source = &hit["_source"];
    let bytes32 = |value: &Value| -> Result<[u8; 32], SearchError> {
        let bytes = hex::decode(value.as_str().ok_or_else(invalid)?).map_err(|_| invalid())?;
        bytes.try_into().map_err(|_| invalid())
    };
    let channel_id = match source["channel"].as_str() {
        Some(id) => Some(id.parse().map_err(|_| invalid())?),
        None => None,
    };
    Ok(SearchHit {
        event_id: bytes32(&source["event_id"])?,
        kind: source["kind"]
            .as_i64()
            .and_then(|kind| i32::try_from(kind).ok())
            .ok_or_else(invalid)?,
        pubkey: bytes32(&source["author"])?,
        channel_id,
        created_at: source["created_at"].as_i64().ok_or_else(invalid)?,
        rank: hit["_score"].as_f64().unwrap_or(0.0) as f32,
    })
}

/// Whether an HTTP status means OpenSearch refused the request itself: a
/// malformed or too large request. Auth failures, a missing index (404),
/// throttling (429) and server errors are the cluster's state, not the
/// request's.
fn rejects(status: u64) -> bool {
    matches!(status, 400 | 413)
}

/// The response body, or an error naming the status.
async fn checked(response: Response) -> Result<Value, SearchError> {
    let status = response.status_code();
    if !status.is_success() {
        let description = format!("HTTP {}", status.as_u16());
        return Err(if rejects(u64::from(status.as_u16())) {
            SearchError::Rejected(description)
        } else {
            SearchError::Engine(description)
        });
    }
    response.json().await.map_err(transport_error)
}

fn transport_error(error: opensearch::Error) -> SearchError {
    if error.is_timeout() {
        SearchError::Engine("request timed out".into())
    } else {
        SearchError::Engine("request failed".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn large_document(byte: u8) -> SearchDocument {
        SearchDocument {
            event_id: [byte; 32],
            kind: 9,
            pubkey: [0; 32],
            channel_id: None,
            created_at: chrono::DateTime::from_timestamp(0, 0).expect("epoch"),
            content: "x".repeat(MAX_BULK_BYTES * 5 / 8),
            channel_type: None,
            author_is_agent: false,
            thread_root: None,
            mentions: Vec::new(),
        }
    }

    /// When a batch goes out as several requests and a later one fails, the
    /// items the cluster already applied are reported as written, so their
    /// rows are completed instead of being sent again forever.
    #[tokio::test]
    async fn a_failed_later_request_keeps_the_items_already_applied() {
        let requests = Arc::new(AtomicUsize::new(0));
        let app = axum::Router::new()
            .fallback({
                let requests = Arc::clone(&requests);
                move |_body: axum::body::Bytes| async move {
                    match requests.fetch_add(1, Ordering::SeqCst) {
                        0 => (
                            axum::http::StatusCode::OK,
                            json!({ "errors": false, "items": [] }).to_string(),
                        ),
                        _ => (
                            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                            "{}".to_string(),
                        ),
                    }
                }
            })
            .layer(axum::extract::DefaultBodyLimit::disable());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let url = format!("http://{}", listener.local_addr().expect("address"));
        let server = tokio::spawn(async move { axum::serve(listener, app).await });
        let index = SearchIndex::new(&OpenSearchConfig {
            url,
            basic_auth: None,
            aws_sigv4: None,
            index_prefix: DEFAULT_INDEX_PREFIX.into(),
        })
        .expect("client");
        let documents = [large_document(1), large_document(2), large_document(3)];

        let outcome = index
            .write(
                CommunityId::from_uuid(uuid::Uuid::nil()),
                &documents,
                &[],
                1,
            )
            .await
            .expect("the first request was applied");
        server.abort();

        // Mutation: propagate a later request's error → RED.
        assert_eq!(outcome.failed, [[2; 32], [3; 32]]);
        assert_eq!(outcome.rejected, 0);
        assert_eq!(requests.load(Ordering::SeqCst), 2, "the rest is not sent");
    }

    #[test]
    fn a_batch_over_the_request_limit_is_split_in_order() {
        // Mutation: never start a new run → RED.
        assert_eq!(
            request_ranges([4, 4, 4, 9, 1].into_iter(), 8),
            [0..2, 2..3, 3..4, 4..5]
        );
    }
}
