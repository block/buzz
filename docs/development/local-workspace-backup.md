# Local workspace backup and restore

These scripts back up the local Buzz development PostgreSQL database and the
`buzz-media` MinIO bucket. They deliberately do not copy `.env`, signing keys,
desktop keychains, Redis, or other secrets and ephemeral state.

## Create a backup

Choose an existing, absolute directory outside the Buzz repository. The script
creates a private timestamped child directory and prints its path:

```bash
just backup-local-workspace /Users/me/BuzzBackups
```

The backup contains a PostgreSQL custom-format archive, a MinIO object mirror
and inventory, and a versioned manifest plus SHA-256 checksums. Its directory is
mode `0700`; files are inaccessible to group and other users. A failed backup
does not contain a completed manifest and must not be restored.

For the most consistent snapshot, stop local relay, desktop, mobile, web, and
CLI processes that can write data before running the backup. The database dump
itself uses a transactionally consistent PostgreSQL snapshot. Every Docker
stage has a bounded runtime and terminates its process group if that deadline
is exceeded.

## Restore a backup

Stop local Buzz applications and relay processes first. Restore is destructive:
it replaces database objects and removes MinIO objects absent from the backup.
The script validates the backup, copies it without following links into a
private temporary directory, validates that staged snapshot, then requires
typing `RESTORE`:

```bash
just restore-local-workspace /Users/me/BuzzBackups/buzz-local-workspace-20260724T010203Z
```

Before confirmation or any Docker mutation, the restore script requires an
absolute backup path outside the main checkout and every registered linked
worktree. Paths are resolved canonically, and symbolic links anywhere in an
archive are refused. The private staged copy is the only copy consumed during
restore.

After confirmation, the script fails closed if the Compose project contains an
unknown service, a known host writer is running, or PostgreSQL has an unexpected
session. It stops every present known Compose writer and temporarily rotates the
local database role credential so stopped clients cannot reconnect during the
destructive PostgreSQL and MinIO restore. Known writers stay stopped until
migrations and readiness checks succeed. Every Docker, validation, restore,
migration, and readiness stage has a deadline; a timeout sends TERM and then
KILL to the entire stage process group and aborts subsequent work.

The common `BUZZ_LOCAL_WORKSPACE_TIMEOUT_SECONDS` environment variable changes
the default 300-second deadline. Individual stages can be tuned with
`BUZZ_LOCAL_WORKSPACE_VALIDATION_TIMEOUT_SECONDS`,
`BUZZ_LOCAL_WORKSPACE_COPY_TIMEOUT_SECONDS`,
`BUZZ_LOCAL_WORKSPACE_DOCKER_TIMEOUT_SECONDS`,
`BUZZ_LOCAL_WORKSPACE_DATABASE_TIMEOUT_SECONDS`,
`BUZZ_LOCAL_WORKSPACE_MINIO_TIMEOUT_SECONDS`,
`BUZZ_LOCAL_WORKSPACE_MIGRATION_TIMEOUT_SECONDS`, and
`BUZZ_LOCAL_WORKSPACE_READINESS_TIMEOUT_SECONDS`.

For non-interactive operator automation, invoke
`scripts/restore-local-workspace.sh ABSOLUTE_BACKUP_PATH --confirm` only after
performing the same explicit human approval in the surrounding workflow.
