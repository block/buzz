# Buzz Agent Handoff Pilot Plan
Created: 2026-07-24

## Purpose

Prove whether Buzz makes AI-agent work easier to delegate, inspect, resume, and trust.

This pilot uses Buzz as an agent-handoff workbench around the existing GitHub/Codex workflow. It does not try to replace Slack, GitHub, CI, or PR review during the first pass.

## Grounding

- Buzz is positioned as a self-hostable workspace where humans and AI agents share the same rooms, and where messages, reactions, workflow steps, review approvals, and git events are signed events in one log. See `README.md`.
- The source pilot path is `just dev`, which starts the relay and real Tauri desktop app. `desktop-dev` is browser-only and should not be treated as the full pilot. See `README.md` and `CONTRIBUTING.md`.
- The repo ships agent-facing surfaces today: `buzz-cli`, `buzz-acp`, `buzz-agent`, `buzz-dev-mcp`, workflow support, Git events, and Git hosting. See `README.md` and `AGENTS.md`.
- `buzz-cli` is explicitly designed for agent reads/writes with JSON output. It can list channels, send messages, and read threads when `BUZZ_RELAY_URL` and `BUZZ_PRIVATE_KEY` are set. See `AGENTS.md` and `crates/buzz-cli/TESTING.md`.

## Scope

### In Scope

- Run Day 0 setup plus five active solo-first pilot days.
- Use one local Buzz community backed by the source checkout.
- Keep GitHub canonical for branches, commits, PRs, CI, and upstream sync.
- Use Buzz as the place where agent tasks are requested, linked, summarized, and review-tracked.
- Capture enough repeatable process that a second human could join later.

### Out Of Scope

- Replacing Slack for general team communication.
- Replacing GitHub for PRs, branch protection, releases, or CI.
- Depending on workflow approval gates, mobile clients, push notifications, or hosted multi-tenant deployment.
- Storing secrets, private keys, or sensitive production data in pilot channels.
- Building new Buzz product features unless the pilot exposes a blocker that must be fixed.

## Success Criteria

The pilot succeeds if, by the end of Day 5, at least three counted agent tasks have a complete Buzz handoff trail and the trail makes resuming or auditing the work easier than terminal-only history.

Counted tasks must include at least one artifact-producing repo task, one interrupted/resume task, and one review or decision task. At most one setup/support task may count toward the three-task threshold, and replayed support simulations do not count.

A complete Buzz handoff trail means the thread contains:

- The root request using the thread template.
- A start reply naming what the agent inspected first.
- At least one checkpoint for the main finding, decision, or fork.
- Links to changed artifacts, branches, commits, PRs, screenshots, or local plan docs.
- Verification evidence or a clear statement that verification was not run.
- A closing reply with outcome, changed artifacts, remaining risks, and next owner.
- Proof that the thread can be read back through `buzz-cli` or the project-local Buzz launcher.

Use this scorecard:

| Signal | Target |
| --- | --- |
| Time to rehydrate context | Under 5 minutes from Buzz thread to useful next action |
| Copy-paste reduction | Fewer repeated prompt/context blocks across Codex sessions |
| Traceability | Each task has request, agent summary, artifact links, and outcome in one thread |
| Confidence | You can answer "what did the agent do and why?" from Buzz without reading raw terminal scrollback first |
| Friction | Fewer than two setup or usage issues severe enough to stop a task |

Friction severity:

- Minor: workaround found in under 10 minutes.
- Major: blocks a task for 30 minutes or requires a fallback mode.
- Severe: prevents Buzz read/write for the day, risks data exposure, or forces the pilot to stop.

## Operating Model

### Roles

- Pilot owner: Steve.
- Human reviewer: Steve, acting as reviewer/operator until the flow is stable.
- Agent operators: Codex and any other configured coding agent.
- System of record for code: GitHub.
- System of record for agent handoff context: Buzz pilot channels.

### Channels

Create four pilot channels:

| Channel | Purpose | Keep It Clean By |
| --- | --- | --- |
| `#buzz-pilot` | Meta channel for setup notes, pilot decisions, and daily scorecard | One daily summary thread |
| `#install-support` | Setup, startup, relay, auth, and desktop launch issues | One issue per thread |
| `#repo-review` | Repo/code review tasks and patch discussion | Link branch, commit, or PR in the root post |
| `#agent-runs` | Live agent task requests, updates, and handoff summaries | One agent task per root thread |

If one channel becomes noisy, split later. Do not add more channels in week one unless a real task cannot be tracked cleanly in the four above.

### Trust And Write Boundaries

Treat signed events as attribution unless Buzz explicitly proves an authorization boundary. During the pilot, keep permissions procedural and conservative:

| Actor | Allowed Pilot Actions |
| --- | --- |
| Steve | Create channels, create task roots, review outcomes, decide adoption gates |
| Codex and other agents | Reply in task/support threads with starts, checkpoints, summaries, and artifact links |
| `buzz-cli` or local launcher | Read channels and threads; post only scoped task/support replies |
| Local relay | Stay local-only unless intentionally reconfigured |
| GitHub | Remain canonical for branches, commits, PRs, CI, and final code review |
| Future teammate | Join only after the teammate invite gate is met |

Agents must not post credentials, approval decisions, branch-protection decisions, or canonical PR review state into Buzz. Link GitHub for those instead.

## Handoff Protocol

Every agent task starts as a Buzz thread before the agent does substantial work.

### Thread Template

Use this shape for each task root post:

```markdown
Goal:

Repo / branch:

Starting context:

Constraints:

Expected artifact:

Definition of done:

Links:
```

Keep the post short enough that an agent can read it in one pass. Put long logs, prior analysis, or screenshots in replies.

### During The Task

- Post one reply when the agent starts, naming what it will inspect first.
- Post a checkpoint when the agent finds the main cause, decision, or fork.
- Post links to local plan docs, branches, commits, PRs, or screenshots as they appear.
- If the task moves to GitHub, keep discussion and final code review canonical there, then link the PR back into the Buzz thread.

### Closing The Task

Close each task thread with:

```markdown
Outcome:

Changed artifacts:

Verification:

Remaining risks:

Next owner:
```

The closure post is the artifact that should let a later agent or human resume without rereading terminal history.

### Task-Thread CLI Smoke Test

Use the project-local Buzz launcher or `buzz-cli` against an active task thread to prove agent-readable context:

- List the pilot channels.
- Post one scoped message to the active task thread.
- Read the same thread back in compact or JSON form.
- Record whether the output was readable enough for a later agent to resume.

## Pilot Safety Rails

### Credential Handling

- Use a disposable pilot identity and key.
- Store private keys outside Buzz, GitHub, tracked files, plan docs, screenshots, and Codex prompts.
- Refer to pilot identity by public key, role, or display name only.
- Do not paste `BUZZ_PRIVATE_KEY`, `BUZZ_AUTH_TAG`, `.env` contents, auth headers, tokens, cookies, or private keys into Buzz.
- Rotate or delete pilot credentials when the pilot ends or if a secret may have leaked.

### Log And Screenshot Redaction

Before posting logs, command output, screenshots, or support notes into Buzz, check for:

- `.env` values, private keys, auth tags, tokens, cookies, and auth headers.
- Shell history that includes exported secrets.
- Local paths, usernames, or machine details that should not become shared context.
- Raw stack traces or config dumps that include credentials.

### Retention And Cleanup

- Identify where local relay data is stored before inviting another person.
- Keep only non-sensitive pilot tasks in Buzz.
- Export useful summaries into docs or GitHub before purging local pilot data.
- Purge local pilot data if a sensitive artifact is posted by mistake.
- Rotate or delete the pilot key at the end of the pilot.

### Fallback Modes

Use the highest-fidelity mode that works:

| Mode | Use When | Minimum Bar |
| --- | --- | --- |
| Full desktop pilot | `just dev`, relay, desktop, and CLI all work | Desktop connects to `ws://localhost:3000` and CLI can read/write |
| CLI-only pilot | Desktop is flaky but relay and CLI work | `buzz-cli` or local launcher can post and read the active thread |
| Markdown emergency capture | Buzz cannot read/write within 30 minutes | Capture the same template in a local markdown note, then backfill or stop |

If Buzz cannot read or write for 30 minutes during a task, record the blocker and stop that day's pilot instead of grinding through setup pain.

## Week-One Plan: Day 0 Setup Plus Five Active Days

### Day 0: Baseline The Environment

- Start the full source pilot with `just dev`.
- Confirm the desktop app connects to `ws://localhost:3000`.
- Confirm the relay is bound only to localhost or a trusted local interface before posting potentially sensitive context.
- Confirm `desktop-dev` is only used for fast UI preview, not pilot validation.
- Build or locate `buzz-cli`.
- Mint or identify disposable pilot credentials, then verify `BUZZ_RELAY_URL` and `BUZZ_PRIVATE_KEY` work for basic channel and message reads without exposing private values.
- Confirm which fallback mode is available if the desktop gets stuck.
- Create the four pilot channels.
- Write the first `#buzz-pilot` daily summary thread with the environment state.
- Run a channel-level CLI check by listing the pilot channels and reading the first `#buzz-pilot` daily summary thread.

### Day 1: First Handoff Task

- Choose a small real task, preferably "review current Buzz startup/pilot docs and summarize whether a new user can follow them."
- Create the task in `#agent-runs` using the thread template.
- Run the agent work normally in Codex, but copy the start, checkpoint, and close summaries into Buzz.
- Use the task-thread CLI smoke test, even if the desktop UI is also open.
- Score whether Buzz reduced context reconstruction compared with a normal Codex-only task.

### Day 2: Interrupted-Session Test

- Start a second agent task and intentionally pause before completion.
- Resume from only the Buzz thread plus repo state.
- Measure time to rehydrate context.
- Record what information was missing from the thread template.
- Update the template in `#buzz-pilot` if needed.

### Day 3: Repo Review Task

- Use `#repo-review` for one review-shaped task.
- Root post should link the branch or local commit under review.
- Keep GitHub canonical for any final review comments or PR state.
- Close the Buzz thread with the review result and GitHub link.

### Day 4: Install/Support Simulation

- Use `#install-support` to capture one real or replayed setup problem.
- Include symptom, environment, command attempted, observed error, and fix.
- Redact logs and screenshots before posting.
- Ask the agent to summarize the fix as a reusable support note.
- Judge whether the support thread would help a future installer.

### Day 5: Evaluate And Decide

- Review all completed handoff threads.
- Score each success signal.
- Confirm the three counted tasks meet the required task mix.
- Confirm there are no unresolved P1 setup or security blockers.
- Decide one of:
  - Continue solo pilot for another week with tighter templates.
  - Invite one teammate for a small collaboration pilot.
  - Pause Buzz adoption until a blocker is fixed.
  - Switch focus from handoffs to workflow automation.

## Measurement Routine

End each day with a short `#buzz-pilot` reply:

```markdown
Tasks attempted:

Best handoff moment:

Worst friction:

Time saved or lost:

Rehydration start/end time:

How I would have resumed without Buzz:

Missing context:

Template changes:

Tomorrow's adjustment:
```

Do not over-measure. The point is to decide whether Buzz creates practical delegation confidence, not to produce a performance report worthy of a consulting slide deck.

## Risks And Mitigations

| Risk | Mitigation |
| --- | --- |
| Buzz becomes another inbox | Keep only four channels and one root thread per task |
| Terminal work still contains the real context | Require start, checkpoint, and close posts for every task |
| GitHub and Buzz disagree | GitHub remains canonical for code state; Buzz links to it |
| Setup issues dominate the pilot | Use fallback modes and stop after 30 minutes without Buzz read/write |
| Sensitive data leaks into local relay history | Use disposable credentials, non-sensitive tasks, redaction checks, and cleanup |
| The agent cannot reliably read Buzz context yet | Require CLI readback proof for complete trails |
| Agents post into the wrong place | Keep agent writes limited to scoped task/support replies |
| Pilot graduates too early | Require task mix, clean handoffs, setup docs, and no unresolved P1 blockers |

## Decision Rules

Continue the pilot if Buzz improves context recovery or auditability on at least two of three counted tasks.

Invite another person only if all of these are true:

- Two clean handoff threads are complete.
- One interrupted/resume task succeeds from Buzz plus repo state.
- Setup steps and fallback modes are documented.
- Existing pilot threads contain no known secrets.
- Pilot credentials have been rotated or confirmed disposable.
- Channel purposes and agent write boundaries are clear.
- There are no unresolved P1 setup or security blockers.

Do not pursue workflow automation until agent handoff threads are consistently useful. Automation magnifies process quality; it does not repair unclear handoffs.

Do not replace Slack or GitHub until Buzz proves a narrower job better than the incumbent workflow.

## Immediate Next Actions

1. Launch the full source pilot with `just dev`.
2. Confirm relay binding, disposable credentials, and redaction rules.
3. Create the four pilot channels.
4. Create the first `#buzz-pilot` daily summary thread.
5. Run the channel-level CLI check.
6. Pick the first task for `#agent-runs`.
7. Run the task-thread CLI smoke test.
8. After the task closes, score whether Buzz made the handoff easier to inspect or resume.
