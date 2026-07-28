# buzz-search

Postgres full-text search (FTS) index and query support.

**Key responsibilities:**
- Maintaining the `search_tsv` tsvector column and GIN index
- Building FTS queries from NIP-50 search filters
- Excluding privacy-sensitive event kinds from indexing
- Tenant-scoped search (by `community_id`)

**Related:**
- [Search](../concepts/search)
- [buzz-db](buzz-db) — shares the Postgres connection
- [NostrProtocol](../concepts/nostr-protocol)
