# Search

Buzz uses Postgres full-text search (FTS) for searching events. There is no external search service — all search is handled within Postgres.

- Events are indexed into a `search_tsv` column (a Postgres `tsvector`) via a GIN index
- Search supports NIP-50, the Nostr standard search protocol
- Queries come in via the relay's `REQ` handler with a search filter
- Privacy-sensitive event kinds are excluded at the storage level (not indexed)
- Search is scoped by community (tenant isolation)

**Implementation:** `buzz-search` crate handles indexing and query building over the `search_tsv` GIN index.

**Related:**
- [buzz-search](../components/buzz-search)
- [NostrProtocol](nostr-protocol)
- [MultiTenancy](multi-tenancy)
