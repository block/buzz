# Persona library and condition matrix

Working document. Tracks what each persona is for, which conditions use it, and
what is still unresolved. Update it in the same change that edits a persona —
a stale entry here is worse than no entry, because manifests pin persona
content by hash and a drifted description hides which condition actually ran.

The personas live in `personas/bench/`. The previous generation
(`personas/*-tb.md`, `*-m1.md`) is kept for reference and for the m1 wiring
proof; nothing new should use it.

## 1. What the agents are actually running inside

Every persona is written against these mechanics. They are not incidental — a
persona that ignores them produces a stalled trial, not a worse score. Each was
read out of the source rather than assumed.

**Agents wake only on @mention.** Each agent is a `buzz-acp` process with
`BUZZ_ACP_SUBSCRIBE=mentions`, all subscribed to one shared channel. An agent
runs a turn only when a message @mentions it by its exact display name (the
harness sets each profile name to the agent id). `ignore_self` defaults on, so
an agent's own message never wakes it.

**A message that mentions nobody wakes nobody.** This is the dominant
multi-agent failure mode: every agent asleep, no pending mention, the trial
burning its 900s timeout to a failure. Every persona therefore ends with a hard
rule about who must be mentioned.

**Only the orchestrator ends the trial.** The harness polls the channel as the
user identity and stops when the roster's single `kind: orchestrator` agent
publishes a message starting with `DONE:`
(`container_runtime.py:504` `_wait_for_done`). A worker posting `DONE:`
achieves nothing. Personas for non-orchestrator slots forbid the prefix
explicitly, so a helpful worker cannot end the trial early by accident.

**Round budget resets per wake.** `BUZZ_AGENT_MAX_ROUNDS` caps LLM/tool rounds
per turn, and the counter is local to each `run()` call
(`crates/buzz-agent/src/agent.rs:82`). A solo agent is woken exactly once, so
its cap is its entire budget for the task; a team member gets the same cap per
assignment, across arbitrarily many assignments. At the old default of 32 that
handed every team roughly a 6× compute advantage over the solo baseline —
larger than any prompt-scheme effect the study is trying to measure — and it
failed silently: a turn that hits the cap ends with no message, so the solo
agent never posts `DONE:` and the trial burns its full 900s to a zero.

`DEFAULT_MAX_AGENT_ROUNDS` is now **300**, chosen so no condition realistically
reaches it, which makes the trial timeout and the cost ceiling the binding
constraints — and those apply to every condition equally. Every persona states
the number and tells the agent to publish what it has by round 280, so a cap
hit degrades to a partial report rather than a silent freeze.

**A 12KB `[Base]` prompt is prepended to every persona.**
`crates/buzz-acp/src/base_prompt.md`, composed as
`[Base]\n{base}\n\n[System]\n{persona}`. It is written for a production Buzz
workspace, and in a graded container a good third of it is actively wrong:
`buzz feed get` startup recovery, `RESEARCH/`/`PLANS/`/`AGENTS.md`, git
worktrees and PRs, `core` memory curation, "keep exploration inside your
working directory", "run the full test suite for the package you touched",
"stop and ask if git email is empty", "narrate as you go", and — the one that
kills trials — "publishing is optional and silence is usually correct."

Each persona opens with a byte-identical block that names and overrides these
specifically rather than gesturing at them, because a general "ignore the above"
does not beat a specific MUST. Whether to suppress it outright is settled for
now: keep it, and measure it (A1n, §3).

**Context isolation is a convention, not a mechanism.** Channel history is
injected automatically only for threaded replies or DMs, capped at 12 messages
(`crates/buzz-acp/src/pool.rs:2493`), so a flat top-level message gives its
recipient nothing but that message — *but* `format_context_hints`
(`crates/buzz-acp/src/queue.rs:1305`) appends `Hint: Use buzz messages get
--channel <UUID> for recent messages if needed` to every top-level wake. An
agent that follows the hint sees everything. Personas therefore both mandate
flat posting and explicitly forbid `buzz messages get`/`thread`/`search`/`feed
get`, naming the hint so it can be ignored on purpose. Without that, how much
context each agent had would vary run to run. Threaded-versus-flat remains a
real future axis (§6).

**One filesystem, shared.** All agents exec in the same task container.
Concurrent writers clobber each other, so every persona carries an ownership
rule.

**The harness appends a `## Your team` block** to the persona at launch
(`container_runtime.py:696`): the agent's own id and pubkey, the channel UUID,
the user to report to, and a table of teammates. Personas must not restate any
of it, and refer to it by name.

The table's Role column carries the **manifest's** `role` string, not the
roster kind. That distinction is load-bearing: personas address each other by
job ("the teammate whose Role column reads `critic`"), and in the three-agent
critic condition the implementer and the critic are both `kind: worker`, so a
table rendered from the kind would print two identical rows and leave the lead
unable to tell which teammate edits and which only verifies. Keep manifest
`role` values drawn from the vocabulary the personas use: `solo`, `lead`,
`implementer`, `critic`, `driver`, `navigator`, and — for the `gt/` generation
(§2.1) — `scout` and `worker`.

**A misspelled @mention fails silently.** `extract_at_mentions_with_known`
(`crates/buzz-sdk/src/mentions.rs:107`) resolves names against known members;
an unmatched name yields zero pubkeys, the send still reports success, and
nobody wakes. Every persona says to copy names from the table character for
character and calls this out as the most fragile thing an agent writes. It is
also unfixable from the prompt side in the general case — a transient relay
hiccup during resolution produces the same silent freeze, which argues for a
harness-side quiet-channel watchdog (§5).

## 2. The personas

Every file shares two byte-identical sections — "This trial is not a Buzz
workspace" and "Messaging" — carrying the `[Base]` overrides, the round budget,
the mention rules, and the stdin form of `buzz messages send`. They are
duplicated rather than injected at compose time so that manifest `prompt.sha256`
still covers the whole prompt; the cost is ~600 tokens per persona and the
benefit is that a diff between two personas shows only the intended delta. If
you edit one of those blocks, edit all eleven and verify:

```sh
for f in personas/bench/*.md; do
  awk '/^## Messaging$/,/^  first message of a turn is your report\.$/' "$f" |
    shasum -a 256 | cut -c1-12
done | sort -u   # must print exactly one line
```

| File | Slot | Terminal | Ends trial | Used by |
|------|------|----------|------------|---------|
| `solo.md` | solo | read+write | yes | A1–A3, A1n |
| `lead-delegate.md` | orchestrator | none | yes | B1, B5, C1, C2, C3, C4, C5 |
| `lead-research.md` | orchestrator | read-only | yes | B2 |
| `lead-implement.md` | orchestrator | read+write | yes | B3 |
| `peer-driver.md` | orchestrator | read+write | yes | B4 |
| `peer-navigator.md` | worker | read-only | no | B4 |
| `worker.md` | worker | read+write | no | B1, B2, B5, C1, C2 |
| `worker-chatty.md` | worker | read+write | no | C4 |
| `worker-deep.md` | worker | read+write | no | C5 |
| `worker-divergent.md` | worker | read+write | no | C3 |
| `critic.md` | worker | read-only | no | B3, C2 |

**`solo.md`** — the control. Everything else is measured against it, so it says
plainly that there is one turn and nobody to delegate to, and tells the agent
not to spend rounds narrating.

**`lead-delegate.md`** — pure coordination: no terminal, one assignment per
message, verification assigned to a different worker than the one being
verified. This is the "manager" archetype and the base for most team
conditions.

**`lead-research.md`** — the lead investigates read-only and hands down
pre-scoped edits quoting the lines to change. Tests whether putting the
expensive model's attention on *understanding* beats spending it on
*coordination*. The read-only constraint is what makes it a different condition
rather than a slower solo run.

**`lead-implement.md`** — inverts the usual shape: the strong model does the
work and the second agent exists only to check it. Cheapest way to ask whether
independent verification is worth an extra agent at all.

**`peer-driver.md` / `peer-navigator.md`** — two strong models as equals, one
holding the keyboard. The navigator is read-only both because the filesystem is
shared and because it forces the value to come from judgement rather than from
a second pair of hands. Handoffs are pinned to three moments (before committing
to an approach, on contradiction, after the check passes) so the condition is
not just "two agents talking."

**`worker.md` / `worker-chatty.md` / `worker-deep.md`** — the reporting
granularity axis, and the only axis on which they differ. The three files are
identical apart from the title line and one `## Reporting cadence` section;
`diff worker.md worker-deep.md` should show nothing else, and if it does the
A/B is measuring something other than granularity. The first draft failed this:
deep also had more error-recovery autonomy, a looser reading of scope, and a
self-verification rule the other two lacked, so a deep win would have been
unattributable. All three now share the same failure rule (report verbatim and
stop), the same verify-before-reporting rule, and the same scope language.

`worker.md` completes an assignment and reports once. `worker-chatty.md`
reports wherever the result could change the plan and at least every three
commands, then waits. `worker-deep.md` reports once at the end and is the only
one that explicitly overrides `[Base]`'s "narrate as you go, never go dark",
which would otherwise pull it toward the chatty arm and compress the effect.

The chatty arm is the timeout risk: a report-and-wait cycle costs a worker turn
plus a lead turn, and at 25-45s per medium-effort turn that is roughly 60-110s
per command against a 900s budget. Unbounded, it buys about a dozen commands
for the whole trial. The "at least every three commands, and not for a command
whose outcome was never in doubt" bound exists to keep the arm finishable; even
so, report its timeout rate separately rather than folding it into mean score.

**`worker-divergent.md`** — for the two-angles condition. The whole persona is
about not converging: pursue the assigned approach even if the other looks
better, do not talk to the sibling, report a dead end as a real result.
Includes a stricter file-ownership rule than the other workers, since parallel
divergent work is exactly where two writers collide.

**`critic.md`** — read-only, adversarial by default, verdict-first (`PASS` /
`FAIL`), and explicitly told not to write the fix. Read-only matters for a
reason beyond collisions: the state the critic assesses has to be the state the
grader sees.

## 2.1 The `gt/` generation — goosetown-derived

`personas/bench/gt/` is a second, self-contained persona family, added
2026-07-29 after every team cell in the study came in at or below its matching
solo baseline (B1 0.536 / C1 0.494 against A1's 0.545; C3 0.831 against A2's
0.843 at 64% more money). It is derived from the `goosetown-*` skill set —
`goosetown-orchestrator`, `goosetown-worker`, `goosetown-reviewer`, and the
eight `goosetown-researcher-*` skills, which all share one template.

| File | Slot | Terminal | Ends trial | Used by |
|------|------|----------|------------|---------|
| `gt/gt-lead.md` | orchestrator | **read-only** | yes | G0, G1, G1s, G2, G2s |
| `gt/gt-scout.md` | worker | read-only | no | G0, G1, G1s, G2, G2s |
| `gt/gt-worker.md` | worker | read+write | no | G0, G1, G1s, G2, G2s |

Three deltas against the `lead-delegate.md` family, each chosen because it maps
onto something the measured numbers say went wrong. None of them is a style
edit.

**The lead cannot write.** `lead-delegate.md:40` says "Do the work directly when
that is the shorter path" and frames delegation as something that must earn its
round trip. That describes a solo agent with an expensive habit, and it is what
C3 measured: score at solo-opus level, input tokens at 1.81× solo opus.
`gt-lead.md` replaces the trade-off with a boundary — reads are the lead's,
every byte written is a teammate's — and carries **no trivial-write exception**,
because the exception is the loophole the old persona fell through. Enforcement
is prose only: there is no per-seat tool gating (`container_runtime.py:297`
gives every agent the same MCP toolset), so "read-only" in the table above is a
rule the persona states, not a sandbox.

**A read-only recon seat exists.** This is new to the study. Every previous
teammate could write, so any two of them were a potential collision and the lead
had to serialise — which is why the team cells effectively ran in series and
paid round trips for it. `gt-scout.md` carries a hard read-only boundary, so
scouts cannot collide with each other or with the worker, and deliberate overlap
between them costs tokens and nothing else. That is what makes a genuinely
parallel recon phase safe, and it is the only reason the `count: 2` scout cells
(G2, G2s) are a different experiment rather than a slower one.

**Reports carry an envelope.** `gt-worker.md` closes with
`STATUS: complete | partial | blocked` plus `DELIVERABLE` / `EVIDENCE` /
`NOTES`; `gt-scout.md` closes with either `BRIEF:` +
`FINDINGS` / `GOTCHAS` / `GAPS` or `VERDICT: pass | pass_with_notes | fail` plus
per-finding severity. Freeform prose made the lead's next decision expensive and
left the failure taxonomy hand-labelled; a fixed shape costs the delegate nothing
and is parseable. All three files also tell the agent to send **the decisive
output, not the transcript**, because every pasted log line is re-sent on every
subsequent round of the trial — the cost mechanism doc 02 §2 is built on.

Two ideas kept from the existing family because they are better than the
goosetown originals. `gt-scout.md`'s verify half keeps `critic.md`'s
**re-derive, do not re-run** rule — `goosetown-reviewer` has no equivalent, and
without it two agents run one script, reproduce one mistake, and both certify
it. And all three files keep the byte-identical "This trial is not a Buzz
workspace" block, which is load-bearing against `[Base]`.

One idea taken from goosetown that has no precedent here and is a **score**
lever rather than a safety one: *"A cancelled writer with 8 of 10 sections on
disk is useful"* (`goosetown-writer/SKILL.md:63`). The grader reads the
container when the clock stops, so `gt-lead.md` and `gt-worker.md` both say to
land the simplest thing that passes the task's own check and refine from there,
rather than assembling the finished answer and writing it once.

### `gt-lead.md` diverges from the shared Messaging block, on purpose

The §2 verification loop globs `personas/bench/*.md` and therefore does not
reach `gt/`, which is correct: **`gt-lead.md`'s Messaging section is
deliberately different.** The shared block says "Every turn you take ends with
exactly one published message". `gt-lead.md` says *at least* one, and adds a
bullet stating that dispatching two teammates means two messages in the same
turn.

That is not a drift to be tidied up. A message wakes exactly one agent, so
"exactly one message per turn" caps a lead at one delegate in flight and
collapses G2/G2s into serial recon — the whole thing they exist to measure. The
invariant that actually matters is the anti-freeze one (never end a turn having
woken nobody), and the reworded bullet preserves it. `gt-scout.md` and
`gt-worker.md` keep "exactly one", since they only ever report to the lead.

Also inherited rather than re-derived: `gt-lead.md` repeats
`lead-delegate.md:64`'s claim that teammates cannot read channel history. Per §1
that is a convention, not a mechanism — `format_context_hints` tells every woken
agent how to fetch history, and no persona in either family forbids it. The
statement is kept verbatim so the G1-vs-C3 comparison differs only where
intended, but read it as an instruction to write self-contained assignments,
not as a guarantee about what a delegate can see.

## 3. Condition matrix

Naming: `tb-<size>-<scheme>-<models>`. Model shorthand: `opus` = Opus 5,
`sol` = GPT-5.6 Sol, `luna` = GPT-5.6 Luna. All at medium thinking effort
(`container_runtime.THINKING_EFFORT`).

### Tier 1 — solo baselines (persona held constant, model varies)

| ID | Condition | Model | Persona |
|----|-----------|-------|---------|
| A1 | `tb-solo-luna` | luna | `solo.md` |
| A2 | `tb-solo-sol` | sol | `solo.md` |
| A3 | `tb-solo-opus` | opus | `solo.md` |
| A1n | `tb-solo-luna-nobase` | luna | `solo.md`, `[Base]` off |

These are the reference points for every cost and score claim. Run them first
and run them at the same `k` as everything else.

**A1n is the `[Base]` sensitivity check**, not a headline condition. Its
manifest is byte-identical to A1's except `include_platform_prompt: false`,
which sets `BUZZ_ACP_NO_BASE_PROMPT=1` and suppresses buzz-acp's ~12KB
production-workspace section. Any score, cost, or token delta between A1 and
A1n is attributable to that section alone, which is what turns "every persona
overrides `[Base]` in prose" from an assertion into a measurement. Run it once,
at the same `k`, and report the delta as a caveat. A1 stays the baseline: the
study is about Buzz, and `[Base]` is what a real Buzz agent receives.

### Tier 2 — two agents

| ID | Condition | Scheme | Orchestrator | Worker |
|----|-----------|--------|--------------|--------|
| B1 | `tb-2-delegate-opus-luna` | delegate | opus / `lead-delegate.md` | luna / `worker.md` |
| B2 | `tb-2-research-opus-luna` | research | opus / `lead-research.md` | luna / `worker.md` |
| B3 | `tb-2-verify-opus-luna` | implement+critic | opus / `lead-implement.md` | luna / `critic.md` |
| B4 | `tb-2-peer-opus-sol` | peer | opus / `peer-driver.md` | sol / `peer-navigator.md` |
| B5 | `tb-2-delegate-opus-sol` | delegate | opus / `lead-delegate.md` | sol / `worker.md` |

B1 and B2 are the pair the question "should the smart model research or
delegate?" reduces to — same models, same slots, one persona different. That
is the cleanest comparison in the whole matrix; keep it that way.

B4 versus B1 changes *both* scheme and worker model, so it cannot attribute
anything on its own. B5 is what makes it readable: B1→B5 isolates the worker
model at a fixed scheme, and B5→B4 isolates the scheme at fixed models. If
budget forces a cut, cut B4 before B5 — a peer result with no control is not
publishable.

Optional if budget allows: `tb-2-peer-opus-luna` closes the 2×2.

### Tier 3 — three agents (orchestrator opus, workers luna, unless noted)

| ID | Condition | Scheme | Roster |
|----|-----------|--------|--------|
| C1 | `tb-3-delegate-opus-2luna` | delegate | `lead-delegate.md` + 2× `worker.md` |
| C2 | `tb-3-critic-opus-2luna` | manager+worker+critic | `lead-delegate.md` + `worker.md` + `critic.md` |
| C3 | `tb-3-divergent-opus-2luna` | two angles | `lead-delegate.md` + 2× `worker-divergent.md` |
| C4 | `tb-3-chatty-opus-2luna` | chatty workers | `lead-delegate.md` + 2× `worker-chatty.md` |
| C5 | `tb-3-deep-opus-2luna` | deep workers | `lead-delegate.md` + 2× `worker-deep.md` |

C1, C4, C5 are one axis with three points — `worker.md` is the midpoint. Read
them together or not at all.

B1→C1 is the team-size axis at a fixed scheme: one worker versus two, same
personas, same models. It is the only place the matrix answers "does a third
agent help?", so it is worth more than any of C2–C5 individually.

### Roster ids

Class ids become agent ids by appending `-<n>`, and
`_classes_by_agent_id` splits on the last hyphen, so a class id must not end in
`-<non-digit>`. Use `lead`, `impl`, `critic`, `worker`. Two workers with the
same persona are one class with `count: 2` (`worker-1`, `worker-2`); two
workers with different personas are two classes with `count: 1`.

## 4. Run order

1. **Smoke, 1 task, k=1**: A1, then B1, then C1. Confirms the solo path, the
   two-agent handoff, and the three-agent fan-out before any money is spent.
   Read the session bundles by hand — the point is to see the actual message
   traffic, not the score.
2. **Tier 1 complete** (A1–A3). Baselines before anything is compared to them.
3. **B1, B2, B5** — the scheme and model axes that carry the argument.
4. **C1, C4, C5** — the granularity axis.
5. **B3, B4, C2, C3** — the remaining shapes, budget permitting.

## 5. Open issues that affect these personas

**Round budget — settled.** `DEFAULT_MAX_AGENT_ROUNDS` is 300, high enough that
no condition should reach it, so the trial timeout and cost ceiling bind
instead and every condition faces the same limits. The residual asymmetry is
that a team still gets 300 *per assignment*; the mitigation is that nothing
should get near 300 in the first place. G11 stays open as a **disclosure**
rather than a fix: state the per-turn cap and its reset semantics in the post,
and report total LLM rounds per trial alongside cost so a reader can check that
no condition was actually round-bound. If any condition's round counts cluster
near 300, the comparison at that point is invalid and the cap must go higher.

**Wall-clock, not rounds, is now the binding constraint** — and it is not
symmetric. Every handoff costs a full agent turn (25-45s at medium effort), so
a chatty three-agent condition can exhaust 900s on protocol overhead while
doing a fraction of the solo agent's work. Timeouts also cost the *full* 900s,
so a condition that times out often is penalised twice, in score and in mean
wall-clock. Report timeout rate as a first-class metric per condition, not
folded into the averages, and consider whether 900s is the right budget for the
three-agent tiers before reading anything into their wall-clock numbers.

**A silent-freeze watchdog is worth considering.** Several failure modes end
the same way — a mention that resolved to nobody, a relay hiccup during
resolution, a turn that hit the round cap — with every agent asleep and the
harness polling to timeout. The personas reduce the prompt-side causes but
cannot touch the transport-side ones. A harness watchdog that re-posts the last
message's mention after N seconds of channel silence would convert a class of
silent 900s losses into recoverable ones. It changes the protocol, so it needs
a methodology footnote if adopted — but an uncorrected transient that reads as
"the team failed the task" is the worse distortion.

**`[Base]` — settled, and measured.** Keep it for production parity in every
headline condition; A1n (§3) quantifies what it costs. Suppressing it entirely
via `include_platform_prompt: false` is now a manifest field and part of the
condition hash. The remaining option not taken is a trimmed benchmark base via
`BUZZ_ACP_BASE_PROMPT_FILE`: it would cut roughly 2,500 tokens from every round
of every turn and delete most of what the personas currently spend ~600 tokens
each rebutting. Worth revisiting if A1n shows `[Base]` is materially hurting,
since at that point "production parity" is preserving a known handicap.

**Endpoint names for Opus 5 are not established.** `databricks-live.json`
currently maps only `databricks-gpt-5-6-sol` and `databricks-gpt-5-6-luna`.
Every Tier-2 and Tier-3 condition above assumes an Opus 5 endpoint on the same
gateway; A3, B1–B5 and C1–C5 are blocked until it exists and its list prices
are recorded. Substitute `sol` as the strong model if it does not.

**Persona hashes must be updated with the text.** Manifests pin
`prompt.sha256` and `_verify_artifact` refuses to launch on a mismatch, which
is the intended behaviour — it makes silent prompt drift impossible. Recompute
with `shasum -a 256 personas/bench/<file>` and update every manifest that
references the file in the same change. Current values:

```
critic.md            b253b735a31ae6eb3c1b8ee318e5e83cef9b6821d49eabff8fe14370ba6b00c6
lead-delegate.md     82df9751aeb5235f7122adc06bafdde12084181b0be84b91721e0d1b5e4a14f7
lead-implement.md    8a1baf78170334ad24510255a0a48af551afcb61646c023e44e95ea8efe37d59
lead-research.md     fc67d8ae2ca96cbd1c0133b5c16bd4ac5700cec9927535168f7ce784dbadbb6a
peer-driver.md       4461b3b7e735b9cdf7c30646a6a54d6b429879c37ea3eb4c8b29b2daceaa7a1d
peer-navigator.md    ec0758b9982cfc499af622be244919305a3b480dc06aa13b3389634efed8ea32
solo.md              6997ae38384ea4f84bbdc1422a668be089fbfd2709d412392dbbb4911c565596
worker-chatty.md     6869901289dab1843e6bd47057c87a9d3178febb23c62521d626b39be402eebf
worker-deep.md       e29e08042707741a603a51fc9e1d1e9c45f0d1fbad6c90f900f6dd8413656589
worker-divergent.md  328ef5af5c200cc8a610b13c73c39426c92a056ff9a01018e0ba35c1979666f8
worker.md            7b3332e3d3da384a7c242c1aa34e90a063e7152ddf6de69f8a4e7e4a0b710726
```

## 6. Candidate future axes

Not in the matrix above; each would need its own conditions.

- **Flat versus threaded messaging.** Every persona here pins flat posting. A
  threaded variant injects up to 12 prior channel messages into each recipient,
  which should raise cost and may raise coordination quality. Cleanly testable
  by changing one sentence in each persona.
- **Per-slot thinking effort.** Currently uniform medium
  (`container_runtime.THINKING_EFFORT`). A high-effort lead over low-effort
  workers is the obvious cost-efficiency play. Blocked on G2 — effort is a
  module constant, not a manifest field.
- **Base prompt: trimmed.** A1n covers on/off. The untested third option is a
  ~2KB benchmark-specific base via `BUZZ_ACP_BASE_PROMPT_FILE`, hashed into the
  manifest — keeping the CLI reference and mention mechanics, dropping the
  workspace, memory, and agent-creation sections.
- **Verification budget.** Every lead here verifies once. Verify-twice, or
  verify-only-on-suspicion, is a distinct scheme.
