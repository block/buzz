# Smart Thread Participation

## Status
- Owner: agent
- Last updated: 2026-08-02
- Current phase: complete
- Decision log:
  - Model as a participation policy, not `respond_to=anyone`.
  - Default ON for `subscribe=mentions`; opt out via `--no-thread-participation` or Settings.
  - Agent-side (buzz-acp) is source of truth so mobile and any client benefit.
  - Desktop "Keep addressed agents active" stays as composer UX; complementary, not required.

## Problem

Agents only wake on an explicit `@mention` (`#p` tag). In a thread where you are already talking to an agent, every follow-up still needs the alias. That is noisy and easy to forget.

## Scenarios

1. Human @mentions agent A in a channel. A replies. Human sends a bare reply in that thread. A responds without another @.
2. Same thread, human @mentions person or agent B (and not A). A stays quiet; B (if an agent) may respond via normal mention rules.
3. Human @mentions both A and B. A responds (explicit mention).
4. Top-level message with no @ and no prior participation. A stays quiet.
5. Another agent posts in a shared thread without @A. A does not auto-continue (avoids agent loops). Humans may still @A.
6. Author gate still applies (`owner-only` / `allowlist` / `anyone`).

## Scope

- buzz-acp: thread participation store, admission decision, subscription without `#p` when enabled under mentions mode.
- Config: `--thread-participation` / `--no-thread-participation` / `BUZZ_ACP_THREAD_PARTICIPATION`.
- Desktop Agents settings toggle that injects the env at managed-agent spawn.
- Unit tests for pure admission logic and config/filter wiring.

## Non-Goals

- Intent classification / LLM-based "should I answer?"
- Changing `respond_to` semantics.
- Replacing "Keep addressed agents active" (composer sticky @).
- Per-channel participation policies.
- ~~Persisting active threads across harness restarts~~ (fixed: disk + relay rehydrate).

## Current Evidence

- Default path: `require_mention` on subscription rules + `#p` in REQ (`filter.rs`, `relay.rs`).
- Author gate is separate (`respond_to` in `lib.rs`).
- Plan note in `docs/plans/grok-build-native-acp.md`: smart participation deferred; not `respond_to=anyone`.
- Client sticky mentions: `persistentAgentAudience.ts` only rewrites the composer.

## Requirements

- Behavioral: bare human replies in an active thread wake the participant agent.
- Behavioral: exclusive other mentions suppress auto-continue for this agent.
- Behavioral: explicit self-mention always wakes (subject to author gate).
- Reliability: agent-authored events never auto-continue without @ (loop guard).
- Security: author gate unchanged; DM hardening unchanged.
- Compatibility: `--no-thread-participation` restores classic mention-only.
- UX: desktop toggle under Agents settings; default on; requires agent restart to apply.

## Architecture

```
Event arrives
  → ignore_self
  → author_allowed (respond_to)
  → match kinds/channels (require_mention=false when participation on)
  → participation.admit:
       mentioned? → mark thread active → accept
       exclusive other p-tags? → drop
       root in active_threads && author is human (non-agent)? → accept
       else → drop
  → queue / prompt
```

Active threads: in-memory map of root event id → last activity, TTL 24h, max 256 LRU.

Subscription: when participation is on and `subscribe=mentions` and not `no_mention_filter`, channel REQ omits `#p` so bare thread replies are visible. Local admission replaces the mention gate.

## Implementation Stages

1. `participation.rs` pure logic + store + unit tests.
2. Config flags and summary line.
3. Mentions-mode rules + channel filters: drop require_mention when participation on.
4. Event loop admission + mark-on-accept.
5. Desktop preference + spawn env + settings toggle.
6. README note.

## Testing Plan

- Unit: mention admits + marks root; bare reply in active thread admits for human; exclusive other mention drops; self-mention admits even with others; agent author drops continuation; TTL/LRU eviction.
- Unit: config default true; no_mention_filter leaves firehose open without narrowing.
- cargo test -p buzz-acp participation / config filters.

## Review Plan

- Wrong-problem: confirm this is not just sticky @ in the composer.
- Regression: classic mention-only still works with flag off; `subscribe=all` unchanged.
- Loop risk: agent-authored continuation blocked.
- Firehose cost: only when participation on under mentions mode; local filter drops non-participants early.

## Definition Of Done

- Bare human reply in an active agent thread is admitted without `@`.
- Exclusive other-@ suppresses the prior participant.
- Opt-out works via CLI/env and desktop toggle (after restart).
- Tests pass for the new module and filter wiring.

## Change Log

- 2026-08-02: Plan written; implement default-on participation policy in buzz-acp.
- 2026-08-02: Implemented. ACP admission + ActiveThreads store; desktop Settings toggle + spawn env; unit tests green.
- 2026-08-02: Fixed restart amnesia — persist active threads to disk and rehydrate from relay history on startup.
