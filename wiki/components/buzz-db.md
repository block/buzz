# buzz-db

Postgres event store and data access layer. Uses **sqlx** for runtime-queried database access (no compile-time macros).

**Key responsibilities:**
- Event persistence (idempotent insert by event ID)
- Event querying (by filter, by ID, by kind, by author, by channel)
- Channel CRUD
- Community CRUD
- Search queries over `search_tsv` GIN index
- Connection pooling and migration management (25 SQL migrations, auto-applied on startup)

**Related:**
- [buzz-relay](buzz-relay) — primary consumer
- [buzz-search](buzz-search) — search queries
- [EventPipeline](../concepts/event-pipeline)
