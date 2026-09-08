# Interaction controls and acceptance criteria

Reviewed against vendor documentation on 2026-09-08. The controls below are
**emulated behavioral examples**, not recordings of tests run inside Slack,
Discord or Teams accounts. Their expected outcomes were written separately from
Buzz's implementation. Browser tests mount the production Buzz timeline/card and
substitute the transport; Rust tests exercise the production schema/state code.
Neither substitutes for the PostgreSQL concurrency and Redis integration gates.

## Control examples

| Control and source | Expected behavior in Buzz | Executable evidence |
| --- | --- | --- |
| [Slack buttons](https://docs.slack.dev/reference/block-kit/block-elements/button-element/) | A named action is keyboard-operable; primary/danger distinguish intent; the choice ID is sent. | `interactions.spec.ts`: keyboard approval and authoritative close |
| [Slack interaction acknowledgement](https://docs.slack.dev/interactivity/handling-user-interaction/) | Show immediate pending feedback; prevent repeat submission; show an error if delivery/signing fails; do not claim a recorded answer prematurely. | `Slack form control`: delayed signer failure and retry |
| [Slack form errors](https://docs.slack.dev/surfaces/modals/#display-errors-in-views) | Required fields prevent submission. Failure leaves entered values available to correct/retry. | `forms retain validation`; `Slack form control` |
| [Discord components](https://docs.discord.com/developers/components/reference) | Multi-select obeys min/max. At the maximum, selected items remain removable; extra unselected items are disabled. | `Discord select control` |
| [Discord polls](https://support.discord.com/hc/en-us/articles/22163184112407-Polls-FAQ) | Explicit submission, a visible recorded choice, and the ability to change that choice while voting remains open. Live results do not overwrite an unsubmitted selection. | `Discord control`: recorded vote → draft change → unrelated tally → submit → revised state |
| [Teams poll lifecycle](https://support.microsoft.com/en-us/forms/poll-attendees-during-a-teams-meeting) | Clearly distinguish open voting from closed results; disable controls when voting ends. Buzz waits for a relay-authored result after its local deadline. | `Teams close control`: clock advances with no incoming close, then authoritative close arrives |
| [GroupMe vote changes](https://support.microsoft.com/en-us/groupme/how-do-i-add-a-poll-to-a-groupme-chat) | A changed vote replaces the entire prior selection before closure. A subsequent attempt after closure cannot change the result. | `poll_control_trace_replaces_the_entire_selection_and_keeps_evidence` in `buzz-core` |

Concrete poll control: Alice selects Door + Key, Bob selects Key, then Alice
changes to Hall. Expected counts are respectively `(1,1,0)`, `(1,2,0)` and
`(0,1,1)`, with two respondents and Alice's newer signed event as her evidence.
Duplicate choices, undeclared choices and selections above the maximum are
rejected. Closing the poll freezes those counts.

Intentional differences: Slack/Discord's three-second acknowledgement contracts
belong to their HTTP interaction APIs; Buzz immediately displays pending status
and waits for its existing relay acknowledgement. Buzz uses signed events and
relay-authored state, direct answer replacement instead of Discord's remove-vote
step, and inline forms instead of a modal. V1 has channel-visible answers only.
It does not claim private/anonymous ballot parity, re-opening polls, native
mobile cards or runtime permission prompts.

## General acceptance criteria

- Off by default: relay writes require the flag and persistent relay identity;
  desktop controls require the separate experimental toggle.
- Decision integrity: current membership/role authorization on the writer,
  immutable source signatures, actor-based replacement, deterministic ordering,
  exact-retry idempotency, and transactional source/state/outbox writes.
- Lifecycle: first/quorum/manual/deadline rules, no acceptance after deadline,
  one counted response per actor, and no client-inferred final outcome.
- Accessibility and recovery: labeled native controls, keyboard operation,
  visible pending/error/closed states, retained form input on failure, bounded
  selection, and live updates that preserve drafts.
- Compatibility: ordinary text projections for clients that do not subscribe to
  the custom kinds; exact whole-reply fallback retains original signed evidence.
- Isolation/privacy: channel and tenant boundaries, unauthorized replies remain
  comments, relay-only state, and rejection of unsupported private/secret forms.
- Delivery: bounded expiry work, durable at-least-once outbox, and pruning of
  queued events removed by moderation or addressable replacement.

The experiment must remain gated until native PostgreSQL races and the full
repository CI jobs pass. Passing browser controls alone does not establish
production readiness or completion of the outline's later workflow/harness
phases. See [the protocol review](experimental-interactions.md) for scope.

## Run the controls

```sh
cargo test -p buzz-core --lib interaction::tests
cd desktop
pnpm build:e2e
pnpm exec playwright test --project=smoke interactions.spec.ts
```

## Try the browser preview

```sh
cd desktop
pnpm build:interaction-preview
python3 -m http.server 4173 --bind 127.0.0.1 -d dist-interaction-preview
# Open http://127.0.0.1:4173/interaction-preview/index.html
```

Open **#engineering**. The separate preview includes three examples and a
permanent simulated-data notice. It uses the actual desktop application and
cards with the repository's mock browser bridge. Replies update an in-memory
demonstration state; they are not real relay decisions. Reload to reset. This
entry point is excluded from normal desktop builds.
