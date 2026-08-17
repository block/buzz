# Autonomous Agent Skills

Command Adviser can turn two successful adviser turns with the same bounded task pattern into a conservative text-only checklist. It does not ask the model to approve its own work and it does not grant new tools, credentials, network access, provider changes, release changes, or authority for external action.

## Runtime behaviour

- `30180` stores an immutable, owner-encrypted skill version.
- `30181` stores the owner-encrypted active-version pointer.
- One matching success records evidence but does not create a skill.
- Two distinct matching successes queue a deterministic candidate. Fixed policy, schema, hash, lineage, and regression checks must pass before publication.
- The version is published first. Only after relay acknowledgement is the active pointer published. Only after both acknowledgements is the verified `SKILL.md` materialized.
- A turn retains the skill-version snapshot it started with. A changed projection invalidates the relevant ACP session before the next turn, which then performs a fresh skill scan.
- Two matching failures against an active child version queue a pointer rollback to its passing parent. Immutable versions and evaluation evidence remain available.

The v1 learner is intentionally narrow. It generates one 32 KiB-or-smaller `SKILL.md` without supporting files or model-authored code.

## Local derived state

For a Command Adviser specialist whose experience outbox is:

```text
~/Library/Application Support/xyz.block.buzz.app/experience-outbox/<agent-pubkey>.sqlite3
```

the default learner registry is:

```text
~/Library/Application Support/xyz.block.buzz.app/experience-outbox/<agent-pubkey>.skills.sqlite3
```

The verified active projection is under the selected agent nest. For the
installed Command Adviser profile, that is:

```text
~/.buzz/.agents/skills/learned-<12 lowercase hex>/
```

Development instances use their configured nest (normally `~/.buzz-dev`) in
the same way. Do not assume a home-level `~/.agents` directory.

Each managed directory contains `SKILL.md` and `.skill-version.json`. Ordinary user-authored skill directories are never replaced or deleted. A malformed managed-looking directory is preserved on disk but ignored by the agent loader.

Both SQLite and the managed projection are disposable caches. Signed relay events are authoritative.

## Degraded operation and recovery

`skill_learning_degraded` means capture, publication, materialization, or rebuild could not complete. Adviser work continues; it does not silently treat unacknowledged skill data as active.

On startup, durable pending work is retried before rebuild. When no publication is in flight, the runtime queries the agent-authored version and pointer events scoped to the owner, verifies signatures and encryption, selects the newest referentially valid pointer for each skill, replaces the derived registry, and atomically recreates the active projection. If the relay is unavailable, the prior known-good projection remains in place.

## Acceptance and rollback check

1. Run `scripts/check-autonomous-skills.sh` from an activated Hermit shell.
2. Submit the same harmless bounded task twice to one specialist and allow both turns to complete successfully.
3. Verify the second turn publishes one version and pointer, creates one verified `learned-*` directory, and the next turn starts a fresh session that advertises that version.
4. Record two matching deterministic failures in the test harness and verify the pointer returns to the parent version.
5. Back up the test profile, remove its derived registry and managed projection, restart with the relay available, and verify the rebuilt `SKILL.md` hash matches the pre-removal hash.

Never remove the installed application, workspace database, Keychain item, or user-authored skill directories during this recovery test.

## Installed relay compatibility

The Mac-local relay is a separate LaunchAgent binary at:

```text
~/Library/Application Support/Command Adviser/relay/buzz-relay
```

An application-only install does not replace it. A release that introduces a
new signed event kind must also build and atomically replace this relay binary,
retain a rollback copy, restart `xyz.block.command-adviser.relay`, and verify
`/health` before running the live promotion canary. Otherwise publication fails
closed with `restricted: unknown event kind` even though the new app is valid.

## Live acceptance — 17 August 2026

- Two matching Chief of Staff turns queued `learned-7e7658179e34`.
- The old installed relay rejected kind `30180`; the Phase 4 relay replaced it
  with a rollback copy retained and accepted kinds `30180` then `30181`.
- Publication reached `materialized`, and `SKILL.md` plus
  `.skill-version.json` appeared under the installed `~/.buzz` nest.
- After restart, the next matching observation recorded the active version it
  started with and promoted a deterministic child version, proving between-turn
  reload and continued learning.
- The application was signed with the stable Developer ID identity. Two
  controlled restarts produced no Keychain prompt.
- Shutdown now explicitly drops the Apple workspace-wake subscription before
  the release build's `_exit`; the second restart retained exactly one watcher
  and nine managed ACP processes.
