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

use std::time::Duration;

use buzz_core::CommunityId;
use buzz_db::search_outbox::SearchDocument;
use opensearch::auth::Credentials;
use opensearch::http::response::Response;
use opensearch::http::transport::{SingleNodeConnectionPool, TransportBuilder};
use opensearch::http::Url;
use opensearch::indices::IndicesRefreshParts;
use opensearch::indices::{IndicesCreateParts, IndicesDeleteParts, IndicesExistsParts};
use opensearch::{BulkOperation, BulkParts, OpenSearch, SearchParts};
use serde_json::{json, Value};

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
            .field("index_prefix", &self.index_prefix)
            .finish()
    }
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
    pub async fn ensure(&self) -> Result<(), SearchError> {
        if self.alias_exists().await? {
            return Ok(());
        }
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
                if self.alias_exists().await? {
                    return Ok(());
                }
                return Err(SearchError::Engine("index exists without its alias".into()));
            }
            return Err(SearchError::Engine("index creation rejected".into()));
        }
        checked(response).await.map(drop)
    }

    async fn alias_exists(&self) -> Result<bool, SearchError> {
        let response = self
            .client
            .indices()
            .exists(IndicesExistsParts::Index(&[&self.alias]))
            .send()
            .await
            .map_err(transport_error)?;
        Ok(response.status_code().is_success())
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

    /// Upsert `documents` and delete `removed` for one community, in one
    /// bulk request through the alias. Fails unless every item succeeded;
    /// deleting a document that is not there succeeds. The request requires
    /// the alias, so a missing index fails the write instead of being created
    /// with a dynamic mapping.
    pub async fn write(
        &self,
        community: CommunityId,
        documents: &[SearchDocument],
        removed: &[[u8; 32]],
    ) -> Result<(), SearchError> {
        if documents.is_empty() && removed.is_empty() {
            return Ok(());
        }
        let index = &self.alias;
        let mut operations: Vec<BulkOperation<Value>> =
            Vec::with_capacity(documents.len() + removed.len());
        for document in documents {
            operations.push(
                BulkOperation::index(document_source(community, document))
                    .id(document_id(community, &document.event_id))
                    .index(index)
                    .into(),
            );
        }
        for event_id in removed {
            operations.push(
                BulkOperation::delete(document_id(community, event_id))
                    .index(index)
                    .into(),
            );
        }
        let response = self
            .client
            .bulk(BulkParts::None)
            .require_alias(true)
            .body(operations)
            .send()
            .await
            .map_err(transport_error)?;
        let body = checked(response).await?;
        if body["errors"] != true {
            return Ok(());
        }
        let failed = body["items"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| item.as_object()?.values().next())
            .filter(|result| result.get("error").is_some())
            .collect::<Vec<_>>();
        let reason = failed
            .first()
            .and_then(|result| result["error"]["type"].as_str())
            .unwrap_or("unknown");
        let description = format!("{} bulk items failed ({reason})", failed.len());
        // Only a batch whose every failure is the documents' own fault is
        // rejected; anything else may succeed on a retry.
        let rejected = !failed.is_empty()
            && failed
                .iter()
                .all(|result| result["status"].as_u64().is_some_and(rejects));
        Err(if rejected {
            SearchError::Rejected(description)
        } else {
            SearchError::Engine(description)
        })
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
