# Task 5 Implementer Report

## Result

Implemented manifest-based local workspace backup and restore in commit
`158e4093`.

- Backups require an explicit absolute existing parent outside the repository.
- `umask 077`, mode `0700` directories, and group/other permission removal keep
  artifacts private.
- PostgreSQL is dumped in custom format.
- The `buzz-media` MinIO bucket is mirrored with a content/size inventory.
- A fixed-shape format-v1 manifest and complete SHA-256 checksum set describe
  the archive without copying `.env`, key material, or credentials.
- Restore validates path containment, regular-file constraints, manifest shape,
  checksum coverage, object inventory, and `pg_restore --list` before asking
  for confirmation or mutating services.
- Confirmed restore stops Compose writers, restores PostgreSQL and an exact
  MinIO mirror, applies migrations, and waits for required service readiness.
- Just recipes and operator documentation expose the workflow.

## TDD evidence

The initial mocked test was RED because both production scripts were absent:

```text
not ok - backup script exists and is executable
```

The first implementation made the suite GREEN. A corrective RED then proved
confirmation occurred before invalid-archive reporting; moving validation
ahead of the prompt restored GREEN. A second corrective RED proved the archive
was not yet inspected with PostgreSQL tooling; adding non-mutating
`pg_restore --list` validation restored GREEN.

## Verification

- `bash scripts/tests/local-workspace-backup-test.sh`: passed.
- `bash -n` across the library, two entrypoints, and mocked test: passed.
- `just --list`: parsed successfully.
- `docker compose config --quiet`: passed.
- `git diff --check`: passed before commit.
- Pre-commit hook: passed.

No live round-trip was run because this task must not destructively exercise the
shared local development stack.

## Review corrections

Addressed all four Important findings from `task-5-review.md`:

- Backup and restore destinations are canonicalized and refused when contained
  by the main checkout or any linked worktree reported by Git.
- Restore enumerates Compose services fail-closed, refuses known host writers
  and unexpected PostgreSQL sessions, stops every present known Compose writer,
  and rotates the local database credential during destructive restore to
  prevent reconnects. Writers restart only after migrations and readiness pass.
- Restore rejects symbolic links anywhere in the archive, copies the validated
  source without following links into a mode-0700 staging directory, validates
  that snapshot again, and consumes only the staged PostgreSQL and MinIO inputs.
- Docker, archive validation/copy, backup, restore, migration, and readiness
  stages use configurable deadlines with process-group TERM/KILL escalation and
  non-zero propagation.

The mocked regressions cover main/sibling worktree refusal, symlink rejection,
an archive replacement after initial validation, unknown services, active
database sessions, known host writers, exact stop/check ordering, and hanging
`pg_dump`, `pg_restore --list`, destructive `pg_restore`, and `mc mirror`
commands. Timeout cases assert descendant cleanup and prove that migrations and
readiness do not run after destructive restore failures.

Fresh corrective verification:

- `bash scripts/tests/local-workspace-backup-test.sh`: passed.
- macOS `/bin/bash` 3.2 syntax check across the library, entrypoints, and test:
  passed.
- Hermit `just --list`: parsed successfully.
- `docker compose config --quiet`: passed.
- `git diff --check`: passed.

No live backup, restore, service stop, credential rotation, migration, or
destructive command was run.

## Final rereview correction

The remaining failure-atomicity finding in `task-5-rereview.md` is addressed:
the destructive custom-archive restore now combines `--clean --if-exists` with
explicit `--exit-on-error --single-transaction`. PostgreSQL cleanup and restore
therefore commit together or roll back together, while the existing database
writer lock remains active until `pg_restore` succeeds.

A test-first mocked regression asserted the complete compatible flag sequence.
It failed against the prior command, then passed after the minimal command-line
change. The full mocked suite, macOS Bash 3.2 syntax checks, Hermit Just parsing,
Compose configuration validation, and diff checks were rerun. No live restore
or error-injection test was performed against the shared PostgreSQL service.
