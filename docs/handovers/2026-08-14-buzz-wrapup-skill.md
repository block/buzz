# Buzz Wrap-up Skill Handover

## Verified

- Managed nests install the bundled `buzz-wrapup` skill canonically and expose it in known provider skill directories.
- Explicit `done`, `wrap up`, and `end session` requests trigger the bundled contract.
- Non-trivial sessions create a `WORK_LOGS/` handover and queue an `OUTBOX/session-digests/` export without making Obsidian canonical or required.
- Managed refreshes preserve owner content after the skill marker, and unmarked existing files are never overwritten.
- Rust formatting, 47 focused nest tests, the full desktop Rust suite, skill validation, and an independent audit passed.

## Pending

- Push the branch and open a draft PR.
- The owner-authorized vault exporter remains an external integration. Buzz queues digests but does not directly write a vault.

## Blocked on Diego

- None.
