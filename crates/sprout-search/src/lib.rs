#![deny(unsafe_code)]
#![warn(missing_docs)]
//! Sprout search — Typesense integration for full-text event search.

/// Typesense collection schema management.
pub mod collection;
/// Search error types.
pub mod error;
/// Event indexing helpers.
pub mod index;
/// Search query execution.
pub mod query;

pub use error::SearchError;
pub use query::{SearchHit, SearchQuery, SearchResult};

use sprout_core::event::StoredEvent;

/// Configuration for the Typesense search backend.
///
/// [`SearchConfig::default`] reads from environment variables so that no
/// credentials are ever hardcoded in source:
///
/// | Field        | Environment variable    | Default (dev only)       |
/// |--------------|-------------------------|--------------------------|
/// | `url`        | `TYPESENSE_URL`         | `http://localhost:8108`  |
/// | `api_key`    | `TYPESENSE_API_KEY`     | `sprout_dev_key`         |
/// | `collection` | `TYPESENSE_COLLECTION`  | `events`                 |
///
/// In production, always set `TYPESENSE_API_KEY` explicitly. The fallback
/// value `sprout_dev_key` is intentionally weak and only suitable for local
/// development with a locally-running Typesense instance.
#[derive(Debug, Clone)]
pub struct SearchConfig {
    /// Typesense base URL (e.g. `http://localhost:8108`).
    pub url: String,
    /// Typesense API key.
    pub api_key: String,
    /// Collection name to use for events.
    pub collection: String,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            url: std::env::var("TYPESENSE_URL").unwrap_or_else(|_| "http://localhost:8108".into()),
            api_key: std::env::var("TYPESENSE_API_KEY").unwrap_or_else(|_| "sprout_dev_key".into()),
            collection: std::env::var("TYPESENSE_COLLECTION").unwrap_or_else(|_| "events".into()),
        }
    }
}

#[derive(Debug, Clone)]
/// Typesense search client.
pub struct SearchService {
    client: reqwest::Client,
    config: SearchConfig,
}

impl SearchService {
    /// Creates a new `SearchService` with a default HTTP client.
    pub fn new(config: SearchConfig) -> Self {
        // SAFETY: default builder with only timeout/connect_timeout config cannot fail
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .connect_timeout(std::time::Duration::from_secs(5))
            .build()
            .expect("SAFETY: default builder with only timeout config cannot fail");
        Self { client, config }
    }

    /// Creates a `SearchService` with an explicit HTTP client (useful in tests).
    pub fn with_client(client: reqwest::Client, config: SearchConfig) -> Self {
        Self { client, config }
    }

    /// Idempotent — safe to call on every startup.
    pub async fn ensure_collection(&self) -> Result<(), SearchError> {
        collection::ensure_collection(
            &self.client,
            &self.config.url,
            &self.config.api_key,
            &self.config.collection,
        )
        .await
    }

    /// Indexes a single event (upsert semantics).
    pub async fn index_event(&self, event: &StoredEvent) -> Result<(), SearchError> {
        index::index_event(
            &self.client,
            &self.config.url,
            &self.config.api_key,
            &self.config.collection,
            event,
        )
        .await
    }

    /// Indexes a batch of events. Returns the number successfully indexed.
    pub async fn index_batch(&self, events: &[StoredEvent]) -> Result<usize, SearchError> {
        index::index_batch(
            &self.client,
            &self.config.url,
            &self.config.api_key,
            &self.config.collection,
            events,
        )
        .await
    }

    /// Executes a search query and returns matching results.
    pub async fn search(&self, query: &SearchQuery) -> Result<SearchResult, SearchError> {
        query::search(
            &self.client,
            &self.config.url,
            &self.config.api_key,
            &self.config.collection,
            query,
        )
        .await
    }

    /// Removes an event from the search index by its event ID hex string.
    pub async fn delete_event(&self, event_id: &str) -> Result<(), SearchError> {
        index::delete_event(
            &self.client,
            &self.config.url,
            &self.config.api_key,
            &self.config.collection,
            event_id,
        )
        .await
    }

    /// Checks that the Typesense server is reachable and healthy.
    pub async fn health_check(&self) -> Result<(), SearchError> {
        let url = format!("{}/health", self.config.url);
        let resp = self
            .client
            .get(&url)
            .header("X-TYPESENSE-API-KEY", &self.config.api_key)
            .send()
            .await?;

        let status = resp.status().as_u16();
        if status == 200 {
            Ok(())
        } else {
            let body = resp.text().await.unwrap_or_default();
            Err(SearchError::Api { status, body })
        }
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind};
    use uuid::Uuid;

    async fn typesense_available() -> bool {
        let client = reqwest::Client::new();
        client
            .get("http://localhost:8108/health")
            .header("X-TYPESENSE-API-KEY", "sprout_dev_key")
            .timeout(std::time::Duration::from_secs(2))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    fn make_service(collection: &str) -> SearchService {
        SearchService::new(SearchConfig {
            url: "http://localhost:8108".into(),
            api_key: "sprout_dev_key".into(),
            collection: collection.to_string(),
        })
    }

    fn make_stored_event(content: &str, kind: Kind) -> StoredEvent {
        let keys = Keys::generate();
        let event = EventBuilder::new(kind, content)
            .tags([])
            .sign_with_keys(&keys)
            .expect("signing failed");
        StoredEvent::new(event, None)
    }

    async fn drop_collection(service: &SearchService) {
        let url = format!(
            "{}/collections/{}",
            service.config.url, service.config.collection
        );
        let _ = service
            .client
            .delete(&url)
            .header("X-TYPESENSE-API-KEY", &service.config.api_key)
            .send()
            .await;
    }

    #[tokio::test]
    #[ignore = "requires Typesense"]
    async fn ensure_collection_idempotent() {
        if !typesense_available().await {
            return;
        }
        let collection = format!("events_test_{}", Uuid::new_v4().simple());
        let service = make_service(&collection);
        service.ensure_collection().await.expect("first call");
        service
            .ensure_collection()
            .await
            .expect("idempotency check");
        drop_collection(&service).await;
    }

    #[tokio::test]
    #[ignore = "requires Typesense"]
    async fn index_and_search_roundtrip() {
        if !typesense_available().await {
            return;
        }
        let collection = format!("events_test_{}", Uuid::new_v4().simple());
        let service = make_service(&collection);
        service.ensure_collection().await.unwrap();

        let unique_token = format!("sprout_search_test_{}", Uuid::new_v4().simple());
        let stored = make_stored_event(&format!("hello {}", unique_token), Kind::TextNote);
        let event_id = stored.event.id.to_string();

        service.index_event(&stored).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        let result = service
            .search(&SearchQuery {
                q: unique_token.clone(),
                ..Default::default()
            })
            .await
            .unwrap();

        assert!(result.found >= 1);
        assert_eq!(result.hits[0].event_id, event_id);
        assert!(result.hits[0].content.contains(&unique_token));

        drop_collection(&service).await;
    }

    #[tokio::test]
    #[ignore = "requires Typesense"]
    async fn index_batch_and_delete() {
        if !typesense_available().await {
            return;
        }
        let collection = format!("events_test_{}", Uuid::new_v4().simple());
        let service = make_service(&collection);
        service.ensure_collection().await.unwrap();

        let events: Vec<StoredEvent> = (0..5)
            .map(|i| make_stored_event(&format!("batch event {i}"), Kind::TextNote))
            .collect();
        let count = service.index_batch(&events).await.unwrap();
        assert_eq!(count, 5);

        let stored = make_stored_event("to be deleted", Kind::TextNote);
        let event_id = stored.event.id.to_string();
        service.index_event(&stored).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        service.delete_event(&event_id).await.unwrap();
        service.delete_event(&event_id).await.unwrap(); // idempotent

        drop_collection(&service).await;
    }

    #[tokio::test]
    #[ignore = "requires Typesense"]
    async fn search_with_kind_filter() {
        if !typesense_available().await {
            return;
        }
        let collection = format!("events_test_{}", Uuid::new_v4().simple());
        let service = make_service(&collection);
        service.ensure_collection().await.unwrap();

        let unique = format!("filter_test_{}", Uuid::new_v4().simple());
        let event_k1 = make_stored_event(&format!("{unique} kind1"), Kind::TextNote);
        let event_k42 = make_stored_event(&format!("{unique} kind42"), Kind::from(42u16));
        service.index_batch(&[event_k1, event_k42]).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        let result = service
            .search(&SearchQuery {
                q: unique.clone(),
                filter_by: Some("kind:=1".into()),
                ..Default::default()
            })
            .await
            .unwrap();

        for hit in &result.hits {
            assert_eq!(hit.kind, 1);
        }

        drop_collection(&service).await;
    }
}
