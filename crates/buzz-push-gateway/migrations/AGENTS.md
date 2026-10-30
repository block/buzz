# Schema changes: read first

Read [docs/database.md](../../../docs/database.md) before changing anything
here. The existing `push_gateway_*` tables are also duplicated in the relay
schema (`migrations/` and `schema/`); until that duplication is removed, a
change to them goes there too. The target is for the gateway to own its tables
alone, so put new gateway tables only here. Never edit a merged migration.
