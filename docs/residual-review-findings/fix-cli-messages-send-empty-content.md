# Residual Review Findings

Source: `ce-code-review` run `20260909-000039-a2c85fbf` (mode:agent, local-aligned vs `44316ff`), branch `fix/cli-messages-send-empty-content`, head `f34cd26e`.

Run artifacts: `/tmp/compound-engineering-501/ce-code-review/20260909-000039-a2c85fbf/`

## Actionable (not applied — out of this change's scope)

- P2 / confidence 100 / `crates/buzz-cli/src/commands/messages.rs:903` (agent-native, downstream-resolver) — **`messages edit --content ""` still publishes an empty message.** `cmd_edit_message` runs only `validate_hex64` + `validate_content_size`, so an empty edit passes and `build_edit` accepts it. Desktop routes an empty edit to its delete confirmation (`desktop/src/features/channels/useChannelPaneHandlers.ts:189-196`), so the CLI diverges. The plan defers this path (`docs/plans/2026-09-08-001-fix-cli-messages-send-empty-content-plan.md`, Scope Boundaries). Follow-up: apply the same emptiness decision to `cmd_edit_message` with an `--allow-empty` opt-out, or route it to the delete primitive. No ticket filed in this run; recorded here as the durable sink.

## Report-only observations

- P3 / confidence 100 / `messages.rs:639` (adversarial, advisory/human) — **zero-width-only content still publishes.** `trim()` uses Unicode White_Space, which excludes U+200B, U+2060, U+00AD and U+FEFF, so a pipeline emitting only those bytes publishes a blank message. Plan-deferred; `crates/buzz-sdk/src/builders.rs:1047` uses the same rule, so the CLI is consistent with the SDK.
- P3 / confidence 75 / `messages.rs:633` (agent-native, advisory) — **error contract is prose while a sibling failure in the same function returns JSON.** The mention-preflight failure at `messages.rs:662-671` returns a structured `Usage` payload; the new guard returns prose, so an agent must substring-match to choose a recovery.
- P3 / confidence 75 / `crates/buzz-cli/src/lib.rs:405` (agent-native, advisory) — **`crates/buzz-acp/src/base_prompt.md:27`, the `after_help` examples and `crates/buzz-cli/TESTING.md:210-213` still teach stdin piping without the new contract.**
- P3 / confidence 100 / `messages.rs:624` (agent-native, `pre_existing`) — **the send path buffers stdin unbounded** before `validate_content_size`, unlike the capped reads in `notes set` (`notes.rs:500-509`) and `mem set` (`mem.rs:327-337`).
- anchor 50 / `messages.rs:616` (project-standards) — `pub allow_empty` has no doc comment; suppressed because `mod commands;` is private, `buzz-cli` has no `#![warn(missing_docs)]`, and sibling fields are undocumented.

## Testing gaps (not closed)

- No positive test for AE2: the `--file` exemption is proven only negatively (the guard did not fire before a failing upload). A passing-upload case would need a media-upload route in the fake relay.
- No clap-level test for `--allow-empty` parsing or help output (verified manually).
- No test for `--allow-empty` against a relay that rejects empty content.

## Residual risks

- The relay still accepting empty kind-9 events is a plan assumption; a deferred relay-side rejection would change what `--allow-empty` can promise.
- `crates/buzz-agent/src/agent.rs:139` counts any command text containing `messages send` as a published turn and ignores exit status, so a guard-rejected send does not re-arm the reply guard.
- `messages send-diff --diff -` shares the empty-content class and stays out of scope.
- Caller completeness for "no existing caller sends empty content" rests on a repo-wide search; runtime-assembled commands cannot be enumerated.
