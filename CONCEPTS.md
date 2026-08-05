# Buzz writer-fence concepts

This is a scoped vocabulary for the Buzz VPS writer-fence integration. It is
not a replacement for the cutover runbook or the live deployment receipt.

- **Database writer fence** — Deployment-global PostgreSQL authority that
  admits one active writer by resource, epoch, holder, and unexpired lease.
- **Writer epoch** — Monotonically advanced database generation acquired when
  a relay becomes the holder; an older epoch is stale even if its process is
  still alive.
- **Writer lease** — Short-lived authority renewed by the owning relay and
  rejected after expiry, replacement, or fencing.
- **Writer-pool connection stamping** — In required/enabled mode, applying the
  resource, epoch, and holder settings to every
  PostgreSQL writer connection.
- **Writer-fence guard** — Migration-0027 `ENABLE ALWAYS` DML/TRUNCATE
  enforcement that checks current PostgreSQL writer authority before a durable
  mutation when the server-side
  `buzz_writer_fence_config.required=true` row is enabled. The process
  `BUZZ_WRITER_FENCE_REQUIRED=true` flag is a separate startup gate.
- **External-effect guard** — When installed and writer fencing is enabled,
  explicit lease revalidation before Redis publication, presence or
  connection-control changes, or an external push.
- **Fenced cutover** — Ordered promotion requiring backup and independent
  restore proof, drained old writers, role hardening, lease acquisition and
  renewal, server-side config-row enablement, and readiness verification.
- **Internal health-port readiness** — `/_readiness` retained only on the
  health listener for probes; it is not a public application route.
- **Durable writer-fence Compose overlay** —
  `deploy/compose/writer-fence.override.yml`, which must be explicitly included
  during manual VPS recreation to select the fenced image and environment.
- **Independent restore proof** — Restore validation separate from a backup
  timer, snapshot, or green health endpoint; it is a cutover precondition.
- **Runtime role hardening** — A dedicated fence-owner control role plus a
  non-owner, non-superuser relay role without direct authority-table access.
- **Release identity separation** — Buzz Desktop v0.5.4 identifies the local
  client release; the fenced relay image and overlay identify the VPS runtime.
- **B2 backup reader handoff** — A storage-provider reader identity and handoff
  used for independent backup-object access; it is not the PostgreSQL database
  writer fence and must not be used as a synonym.
