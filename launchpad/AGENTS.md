# launchpad-26/buzz — how we work here

**Read this before filing an issue, opening a PR, or changing anything in this repo.**

This file is the **normative spec** for how work is filed, reviewed and merged in this
fork. Agents: read it in full before your first action in this repository.

Humans usually want [README.md](README.md) first — it covers the same ground with
examples and less rule-text. Where the two disagree, **this file wins**; fix the drift
rather than living with it.

---

## 1. What this fork is for

This repository is a fork of [`block/buzz`](https://github.com/block/buzz), operated by
the launchpad-26 cohort.

**We operate Buzz. We do not develop Buzz.**

That distinction changes almost everything about what work looks like here:

| | `block/buzz` (upstream) | This fork |
|---|---|---|
| Goal | Build the Buzz product | Deploy and run Buzz for rhizomorph |
| Typical change | Rust crates, desktop React, mobile Flutter | Ansible, CI/CD, docs, relay config |
| Typical issue | Feature, product bug | PRD, deployment task, ADR |

The root `CLAUDE.md` and `AGENTS.md` are **upstream's contributor guide**. They will
tell you to run `just ci`, register event kinds in `buzz-core/src/kind.rs`, and open
PRs against `block/buzz`. For deployment, docs, and cohort process work, **that guidance
is wrong, not merely irrelevant.** This file supersedes it for anything under
`launchpad/`, `.github/workflows/launchpad-*`, and all cohort process work.

Genuine upstream product bugs still belong at
[block/buzz/issues](https://github.com/block/buzz/issues).

---

## 2. The one rule

> **Stable knowledge belongs in a document. Active work becomes a GitHub issue.**

Consequences, which are not negotiable:

- No `TODO` comments in code. File an issue.
- No `PLANNED.md`, no roadmap files. Those are issues.
- If something is not true yet, it is an issue. If it is true, it is documentation.
- A decision still being argued is an **ADR issue**. Once made, it becomes
  `launchpad/decisions/ADR-XXXX-slug.md` and the issue closes.

---

## 3. Where cohort files go

Everything cohort-specific lives under `launchpad/`. Upstream owns everything else.

```
launchpad/
  AGENTS.md            this file
  AGENT_PR_TEMPLATE.md  PR body schema for agent-authored PRs
  labels.yml           label source of truth
  sync-labels.sh       applies labels.yml
  decisions/           ADRs, once accepted
  docs/                MkDocs knowledge layer
  deploy/              host configuration and hardening
  upstream-intel/      upstream tracking tooling
```

**Never move or rename upstream files.** Upstream is ~3,800 files and we merge from it
regularly; a rename turns every future merge into manual work.

Two deliberate exceptions, both in `.github/`, both accepted knowingly:

- `.github/ISSUE_TEMPLATE/` — our templates replace upstream's, which pointed
  contributors at `block/buzz`.
- `.github/PULL_REQUEST_TEMPLATE.md` — one added section.

New workflows go in `.github/workflows/` (GitHub requires it) and **must** be named
`launchpad-*.yml` so they never collide with upstream's.

---

## 4. Choosing an issue type

Five types. **Exactly one `type:` label per issue** — a type never modifies another
type.

Work down this list. **The first "yes" wins.** Do not reorder it.

| # | Ask | Yes → | Test |
|---|---|---|---|
| 1 | Is the output a **decision plus rationale**, with nothing in the repo changing when it closes? | **ADR** | A document records a choice; no code or config moves |
| 2 | Does something **exist and behave incorrectly**? | **Bug** | You ran it and observed the failure |
| 3 | Does something **exist and work, but insufficiently**? | **Enhancement** | Behaviour is correct, just not good enough |
| 4 | Does it need **child issues** to finish? | **PRD** | It has acceptance criteria and decomposes |
| 5 | Otherwise | **Task** | One agent, one branch, one PR |

ADR is first on purpose: **decisions masquerade as work.** "Pick a config management
tool" looks like a Task until you notice nothing ships when it closes.

### How the types relate

```
Milestone  (M0, M1)
└── PRD                    the approvable unit; holds acceptance criteria
    ├── Task               executable child: one branch, one PR
    ├── Bug                found while building
    ├── Enhancement        deferred improvement
    └── ADR                an open question the PRD cannot proceed without

ADR ───────────────────────  standalone only when no PRD raised it
```

1. **An ADR is never a work item and never has children.** Work a decision creates is
   filed separately afterwards and linked back.
2. **A PRD's open questions are raised as ADR issues, parented to that PRD.** Use
   `--parent`, exactly as for a Task. An open question that stays in a PRD body is
   invisible on the board, and gets decided by accident inside whichever task hits it
   first — which buries a decision with real consequences in a task nobody reads again.
   An ADR that no PRD raised is filed standalone.
3. **A resolved ADR is written to `launchpad/decisions/ADR-XXXX-slug.md` in the same PR
   that closes its issue.** A decision that exists only in a closed issue is lost to the
   noise. Closing the issue without writing the document is not done. This does not make
   an ADR a work item — no code or config moves; the decision record is the only artifact.
4. **A Task never has children.** If a Task grows children, it was a PRD — relabel it.
5. **Bug and Enhancement** are children of a PRD if found while building it, standalone
   if found later against shipped work.
6. **An Enhancement against unshipped work is a scope change to its PRD, not an
   Enhancement.** Comment on the PRD instead. Without this rule, Enhancement becomes the
   dumping ground for "we didn't finish", and PRDs look done while their gaps live in a
   parallel queue.

### When to raise at all

If the fix is in a file you are already touching and it is small, fix it and note it in
the PR body. Anything else gets an issue. Without a threshold you get either invisible
work or issue spam.

---

## 5. Rules for agents

These are hard constraints, not style preferences.

1. **Draft everything. Approve nothing.** You may write any issue, PR, or ADR in full.
   You may not decide an ADR outcome, approve a PR, or close another agent's escalation.
   Raise concerns; never clear them.
2. **When the type is unclear, file a Task, add `needs-triage`, and say so in the
   Objective.** Never guess silently between PRD and Task — misfiling a PRD as a Task
   hides an approval gate.
3. **Add `by:agent`** to every issue and PR you create. Agents run under a human's
   token, so GitHub's author field cannot distinguish us. The label restores that signal.
4. **Never claim a check you did not run.** Do not write "tests pass". Paste the command
   and its raw output. If you could not run something, say so in *Not verified*.
5. **Never invent sections.** Fill the template's fields. If a field does not apply,
   write `N/A - <one-line reason>`.
6. **Do not fabricate.** No invented file paths, issue numbers, model names, or command
   output. If you do not know, write that you do not know.

### Filing an issue

There is one specialised form beyond the five types: **Agent workflow proposal**
(`06-agent-workflow.yml`). It is an Enhancement with different prompts — it requires the
specific access an agent needs and its blast radius. Use it for any proposal that an agent
should do something a person does today. Guidance is in #40.

Read the template for your chosen type in `.github/ISSUE_TEMPLATE/` and fill it. The
YAML `description:` of each field tells you what it wants, and each template opens with
an `AGENT INSTRUCTIONS` comment block — read it.

```bash
gh issue create \
  --title "task: add Redis role to the relay playbook" \
  --body-file /tmp/issue.md \
  --label type:task --label area:deploy --label by:agent \
  --parent 4
```

Note `--parent` — that creates a real GitHub sub-issue link. Use it for every Task under
a PRD, and for every ADR raised from a PRD's open questions. Only an ADR that no PRD
raised is filed without one.

Do **not** pass `--type`; that is GitHub's org-level Issue Types feature, which this org
has not configured. **Type is a label.**

### Opening a PR

Read `launchpad/AGENT_PR_TEMPLATE.md`, fill it, submit the filled body:

```bash
gh pr create -F /tmp/pr-body.md --base launchpad --label by:agent
```

Do **not** pass `--template`. The template file is a schema you fill, not a body you
paste.

---

## 6. Branch, commit, PR

```bash
git checkout -b <short-slug> launchpad/launchpad
# work
git commit -s                          # -s is required: DCO check
git push -u launchpad <short-slug>
gh pr create --base launchpad
```

- **`git commit -s` every time.** The DCO check fails any commit without a
  `Signed-off-by` trailer.
- **Conventional commit titles**: `feat(deploy): ...`, `fix(ci): ...`, `docs(...): ...`.
  We squash-merge, so the **PR title** becomes the commit subject on `launchpad`.
- **One issue, one PR.** Use a closing keyword — `Closes #12` — so the board updates on
  merge. If the PR genuinely completes nothing — a plan, one step of a larger task, a
  docs correction — use `Refs #12` instead. Both satisfy the PR body check; only
  `Closes` moves the board, so do not reach for it to make a check go green.
  **Write the reference as plain text, not inside backticks or a code block.** GitHub
  creates no link from a reference inside code, so one written there closes nothing.
- **The `launchpad` branch is protected.** PRs require **at least two approving reviews
  from reviewers with write access**, and you cannot approve your own. The ruleset that
  enforces this is not readable without `admin:org` — `rules/branches/launchpad`,
  `rulesets` and `branches/launchpad/protection` all report nothing. A live PR's
  `reviewDecision` confirms that review is *required* (`REVIEW_REQUIRED`) but exposes no
  count; the figure of two comes from GitHub's merge box on an open PR, which is the only
  place it is stated without admin.
- **Do not force-push during review.** Push new commits instead — force-pushing hides
  what changed from the reviewer.
- **Run `gh repo set-default launchpad-26/buzz` once per clone, before the first
  `gh issue create` or `gh pr create`.** Without it, `gh`'s default-repo resolution
  targets `block/buzz` — the parent repository — for any command with no explicit
  `--repo`. A write aimed at this fork (an issue, a PR comment) silently lands on
  upstream's public tracker instead, and the mistake is not visible until someone
  looks there. The setting lands in the shared `.git/config`, so it covers every
  worktree of the clone, not just the one it was run in.

Before running any git command, activate the toolchain or hooks fail on `PATH`:

```bash
. ./bin/activate-hermit
```

---

## 7. Labels

`launchpad/labels.yml` is the source of truth. Apply it with `./launchpad/sync-labels.sh`.

That script exists and works. If you add a label to the file, run it — do not hand-create
labels in the UI, or the file stops being true.

---

## 8. Security

- **Never open a public issue for a vulnerability.** Use the private advisory link on the
  issue chooser page.
- **This repository is public.** Every file you commit is world-readable. Config is fine;
  credentials never are. Parameterise secrets out of files from the first commit.
- Never add a secret, key, token, or private hostname to a tracked file.
