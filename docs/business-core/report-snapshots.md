# Report snapshots

Snapshot generation freezes scope, rule version, maximum fact watermark,
component totals, source hash, generator and trace. Identical scope/rule/watermark
generation is idempotent. A later snapshot may reference the one it supersedes,
but prior snapshots and rows remain unchanged. Evidence records make every
published number reproducible from facts at or below its watermark.
