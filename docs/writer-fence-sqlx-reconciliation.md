# SQLx v27 writer-fence reconciliation

The VPS recorded migration `0027_writer_epoch_fence` with the historical
SHA-384 checksum
`eabb082f2d06077f9a4a09bd8c674ce3989bfa86b1441644093198c3e0d3f6830ee68601f872faefaeee4e69a93e71f7`.
The checked-in migration is the later server-config variant and hashes to
`013887fa040a1395f0c25ae0af613927f1725ee8ae28bc6ecca7e1262d0220134c65350e78bbd31d38507919ade76210`.

This is not repaired by blindly replacing the ledger value. Migration 0029 is
the explicit, idempotent bridge: it creates the missing config row when needed
and replaces the legacy session-GUC guard with the server-side config guard.
Only after the backup/restore proof and the catalog preconditions pass may the
operator update the v27 SQLx checksum and retain an old -> new reconciliation
receipt.

The VPS remains on `BUZZ_AUTO_MIGRATE=false`; the reconciliation is a
controlled migration-admin operation, not a startup-side repair.

On this deployment, `_sqlx_migrations` remains owned by the bootstrap role and
the runtime role `buzz` has no table or schema-create privilege. The validated
operator path is therefore `buzz-admin migrate` in the PostgreSQL maintenance
network namespace with the bootstrap role; no runtime privilege was widened.

Live evidence on 2026-08-05: v27/v28/v29 checksums match the source ledger,
the canonical guard reads `buzz_writer_fence_config`, epoch 16 is active,
public liveness is 200, public readiness is 404, private readiness is 200,
and backup generation `20260805T013856Z-jt-buzz-staging` completed.
