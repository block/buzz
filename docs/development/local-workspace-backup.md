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
itself uses a transactionally consistent PostgreSQL snapshot.

## Restore a backup

Stop local Buzz applications and relay processes first. Restore is destructive:
it replaces database objects and removes MinIO objects absent from the backup.
The script validates the backup first, then requires typing `RESTORE`:

```bash
just restore-local-workspace /Users/me/BuzzBackups/buzz-local-workspace-20260724T010203Z
```

Before confirmation or any Docker mutation, the restore script requires an
absolute backup path outside the repository and validates the manifest shape,
format version, archive, object inventory, and every checksum. It then stops
Compose services that can write Buzz data, restores PostgreSQL and MinIO, runs
the current migrations, and waits for PostgreSQL, Redis, and MinIO readiness.
Any failed command aborts the operation with a non-zero status.

For non-interactive operator automation, invoke
`scripts/restore-local-workspace.sh ABSOLUTE_BACKUP_PATH --confirm` only after
performing the same explicit human approval in the surrounding workflow.
