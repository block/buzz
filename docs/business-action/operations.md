# Operations

Release default is `BUSINESS_ACTION_MODE=production`; acceptance requires `BUSINESS_ACTION_ACCEPTANCE_ACKNOWLEDGE=Desensitized Acceptance - Production Disabled` and `BUSINESS_ASSIGNEE_RESOLVER=acceptance`. Production additionally requires HTTPS origins, formal authorization enabled, and a production resolver. This build deliberately refuses production workflow writes because those adapters are not installed.

Required configuration includes database/gateway URLs, service credential, allowed origin, catalog path/version, preview and draft TTLs, timezone, active-item limit, per-user rate limit, resolver, and authorization enablement. Secrets belong in the server secret store and must never use a `VITE_` prefix.

Startup applies gateway/action migrations, loads the persisted canonical state, validates the catalog, configures limits, and binds to `127.0.0.1:3012` by default. A persistence failure rolls the in-memory mutation back and returns `PERSISTENCE_UNAVAILABLE`. Health only proves the process is responsive; production readiness also requires formal adapters and real end-to-end verification.
