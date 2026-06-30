---
name: dream
description: >
  Automated memory consolidation. Triggered by a relay-emitted dream-due signal
  when the agent's memory exceeds configured thresholds. Runs at idle time,
  lowest priority, preemptible with abort.
version: 1
---

# Dream Skill — Memory Consolidation

You have been signaled that your memory is over budget. Your job is to consolidate it below threshold by distilling, archiving, or leaving each slug untouched. Work atomically per slug so that an abort at any point leaves memory in a consistent state.

## Constraints

- **Invisible.** Do not post messages to any channel. Do not narrate. This is silent housekeeping.
- **Preemptible.** You may be aborted at any moment. Each slug must be fully processed (or not started) — never half-written.
- **Never hard-delete.** Every slug you remove must first be archived to `dream-archive-*`. The `rm` command publishes a tombstone — use it only after the archive is verified.
- **Preserve rollback.** Archives are your undo mechanism. Never skip the verification step.
- **`core` is special.** You may UPDATE `core` (distill it shorter) but you may never DELETE it. Use `mem set core` — `mem rm core` is rejected by the CLI.

## Algorithm

### Step 1: Inventory

```bash
buzz mem ls --json
```

List all slugs. Exclude any slug whose name starts with `dream-archive-` — these are cold storage and not subject to consolidation.

### Step 2: Recall-First Triage

For each non-excluded slug, read its content:

```bash
buzz mem get <slug>
```

Classify the slug into exactly one operation:

| Op | Meaning | When to use |
|----|---------|-------------|
| `NONE` | Already minimal, leave untouched | Content is load-bearing and cannot be shortened without losing value |
| `UPDATE` | Rewrite to a shorter, distilled form | Content has value but is verbose, redundant, or contains stale sections |
| `DELETE` | Archive and tombstone | Content is entirely stale, superseded, or duplicated elsewhere |
| `ADD` | Create new content | Only if splitting a large slug into smaller ones (rare during consolidation) |

**Triage criteria — ask for each slug:**
1. Is this still relevant to active work? (If no → DELETE)
2. Does it contain completed/shipped items that have no open follow-up? (If yes → those sections are candidates for removal via UPDATE)
3. Can the remaining content be expressed in fewer bytes without losing recall value? (If yes → UPDATE)
4. Is it already minimal? (If yes → NONE)

### Step 3: Execute Operations (atomic per slug)

Process slugs in priority order: DELETE first (biggest byte savings), then UPDATE, then ADD. This maximizes the chance that an abort still leaves you under budget.

#### DELETE operation

```bash
# 1. Capture the original hash BEFORE reading (avoids shell newline issues)
ORIG_HASH=$(buzz mem hash <slug>)

# 2. Read current value and archive to cold storage
buzz mem get <slug> | buzz mem set "dream-archive-<slug>-<YYYYMMDD>" -

# 3. Verify archive is byte-identical
ARCHIVE_HASH=$(buzz mem hash "dream-archive-<slug>-<YYYYMMDD>")
# If ORIG_HASH ≠ ARCHIVE_HASH → STOP. Do not proceed. Move to next slug.

# 4. Tombstone the original
buzz mem rm <slug>
```

#### UPDATE operation

```bash
# 1. Capture the base hash (for later conflict detection and archive verification)
ORIG_HASH=$(buzz mem hash <slug>)

# 2. Archive current value to cold storage (byte-exact pipe, no shell variable)
buzz mem get <slug> | buzz mem set "dream-archive-<slug>-<YYYYMMDD>" -

# 3. Verify archive is byte-identical
ARCHIVE_HASH=$(buzz mem hash "dream-archive-<slug>-<YYYYMMDD>")
# If ORIG_HASH ≠ ARCHIVE_HASH → STOP. Do not overwrite the live slug.

# 4. Read current value for distillation (now safe — archive exists)
CONTENT=$(buzz mem get <slug>)

# 5. Distill CONTENT into DISTILLED_CONTENT (your judgment, guided by Distillation Guidelines)

# 6. Write the distilled version
printf '%s' "$DISTILLED_CONTENT" | buzz mem set <slug> -
```

Use `mem set` rather than `mem patch` for UPDATE — you are rewriting the entire value, not applying a diff. If exit code 5 (write conflict) is returned, another agent wrote to this slug during your dream. Skip it and move on.

#### ADD operation

Use `mem set <new-slug>` to create. Only use ADD when splitting a large slug into focused sub-slugs — never to create net-new content during a dream run.

### Step 4: Verify Budget

After processing all classified slugs, re-run `buzz mem ls --json` and estimate total size. If still over budget, you may do a second pass with stricter distillation — but do not loop more than twice. Two passes is the maximum. If still over budget after two passes, stop. The next dream cycle will continue the work.

## Distillation Guidelines

When rewriting a slug (UPDATE), apply these principles:

1. **Keep decisions, drop narrative.** "We chose X because Y" → keep. "After discussing options A, B, C, we eventually settled on X" → distill to just the decision.
2. **Keep interfaces, drop implementation history.** API shapes, config keys, CLI flags → keep. "First we tried Z, then refactored to W" → drop.
3. **Keep active pointers, drop completed arcs.** Open PRs, pending decisions, blocked items → keep. Merged PRs, shipped features, resolved questions → drop unless they establish a precedent needed for future work.
4. **Preserve source citations.** File paths, line numbers, URLs that ground a claim → keep. They cost few bytes and prevent re-research.
5. **Compress, don't summarize.** The goal is the same information in fewer bytes, not a lossy summary. If you cannot preserve the information in fewer bytes, classify as NONE.

## Safety Rules

1. **STOP on any hash mismatch.** If an archive verification fails, do not proceed with that slug. Move to the next slug.
2. **STOP on any write conflict (exit code 5).** Another agent is actively writing. Skip that slug.
3. **Never write to `core` without archiving first.** Even for UPDATE, the archive step is mandatory.
4. **Never delete `core`.** The CLI rejects `mem rm core`, but do not attempt it.
5. **Never create a `dream-archive-*` slug that overwrites an existing archive.** Use a date suffix. If the dated archive already exists (from a prior aborted run), append a sequence number: `dream-archive-<slug>-<YYYYMMDD>-2`.
6. **Two-pass maximum.** Do not loop indefinitely. If two passes don't bring memory under budget, stop.
7. **No channel messages.** Do not post to any channel, thread, or DM during a dream run. This turn is invisible to humans.
