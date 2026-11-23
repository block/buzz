# Schema changes: read first

Read [docs/database.md](../../../docs/database.md) before changing anything
here. The gateway owns its `ingress_gateway_*` tables alone, in its own
database: a schema change goes in `migrations/` and in the declarative
desired state under `../schema/tables/public/`, and nowhere in the relay
schema. Never edit a merged migration.
