# `launchpad/` — the cohort's working directory

Everything the launchpad-26 cohort adds to this fork lives here. Upstream
(`block/buzz`) owns everything outside this directory.

**New here? Read this file, then [AGENTS.md](AGENTS.md).**

> **Authority:** [AGENTS.md](AGENTS.md) is the normative spec. This README explains and
> illustrates; where the two ever disagree, AGENTS.md wins. Fix the drift rather than
> living with it.

---

## What this fork is

We **operate** Buzz. We don't develop it.

The relay, desktop app and mobile app in this repo are upstream's product. Our work is
deploying that product, running it for rhizomorph, documenting it, and automating the
pipeline around it. So most issues here are deployment, CI/CD, docs and decisions —
not Rust or React.

If you've found a genuine bug *in Buzz itself*, it belongs at
[block/buzz/issues](https://github.com/block/buzz/issues), not here.

---

## Filing an issue

Blank issues are turned off. You pick a type, and the form asks for what that type needs.

| Type | Use it when | Example |
|---|---|---|
| **PRD** | The work needs child issues to finish | "Reproducible hardened deployment from bare Ubuntu" |
| **Task** | One person or agent, one branch, one PR | "Add an Ansible role that installs Redis" |
| **Enhancement** | It exists, works, but should work better | "Deployment playbook takes 12 min; cache the apt step" |
| **Bug** | It exists and behaves incorrectly | "`/health` returns 502 after relay restart" |
| **ADR** | A decision to make and record | "Ansible vs. cloud-init for host config" |

**Not sure?** File a **Task**, add `needs-triage`, and say you're unsure. That's the
correct move, not a failure — a misfiled PRD hides an approval step, which is worse.

**One specialised form.** *Agent workflow proposal* is an **Enhancement** with a
different set of prompts — it asks for the access an agent needs and its blast radius,
because a sandbox can only be scoped to a purpose someone wrote down. It carries
`type:enhancement` + `area:agents-and-automation`. See #40 for the submission guidance.

The full decision procedure — five questions, first yes wins — is in
[AGENTS.md §4](AGENTS.md#4-choosing-an-issue-type).

### The rule that catches people out

> An **Enhancement** against work that hasn't shipped yet is a **scope change to its
> PRD**, not an Enhancement.

Comment on the PRD instead. Otherwise "we didn't finish" quietly becomes a separate
queue, and the PRD looks done when it isn't.

### Worked example

You're building the VPS deployment (PRD #2) and notice Redis isn't installed anywhere.

- Not a Bug — nothing exists to behave incorrectly.
- Not an Enhancement — there's nothing to improve yet.
- Doesn't need children — one playbook role, one PR.

→ **Task**, parent `#2`, labels `type:task` + `area:deploy`.

---

## Labels

15 labels, in [`labels.yml`](labels.yml). Apply changes with
[`./sync-labels.sh`](sync-labels.sh) — that script exists and works, so don't
hand-create labels in the GitHub UI or the file stops being true.

**Type** — exactly one per issue. The template applies it; never add a second. A type
never modifies another type.

`type:prd` · `type:task` · `type:enhancement` · `type:bug` · `type:adr`

**Area** — what it touches. Zero or more.

`area:deploy` · `area:docs` · `area:upstream-intel` · `area:ci` · `area:relay-ops` · `area:process` · `area:agents-and-automation`

**State**

- `needs-triage` — type unclear or required content missing; a human should look
- `needs-decision` — blocked on a decision (auto-applied to ADRs)
- `by:agent` — filed or authored by an AI agent

`by:agent` exists because agents run under a human's token, so GitHub's author field
can't tell you who really wrote it. The label restores that signal — and switches the
PR check into its stricter mode.

---

## Opening a PR

**One issue, one PR.** Use a closing keyword (`Closes #12`) so the board updates on merge.

There are two templates, split by **who wrote the code** — not by issue type:

- **Humans** get [`.github/PULL_REQUEST_TEMPLATE.md`](../.github/PULL_REQUEST_TEMPLATE.md)
  automatically. Nothing to choose.
- **Agents** fill [`AGENT_PR_TEMPLATE.md`](AGENT_PR_TEMPLATE.md) and add `by:agent`.

Agent PRs are asked for more because reviewing them needs more: which model, on whose
instruction, what it decided versus escalated, and — the important one — **what it
couldn't verify**. Reviewing a human PR doesn't need any of that.

Both paths are checked in CI, so structure holds whether the PR came from the browser
or the CLI.

Branch from `launchpad`, commit with `-s` (the DCO check is not optional), and expect to
need one approving review — the branch is protected and you can't approve your own.

---

## What's in this directory

| Path | What it is |
|---|---|
| [`README.md`](README.md) | This file — start here |
| [`AGENTS.md`](AGENTS.md) | Normative spec: types, rules, agent constraints, git workflow |
| [`AGENT_PR_TEMPLATE.md`](AGENT_PR_TEMPLATE.md) | PR body schema for agent-authored PRs |
| [`labels.yml`](labels.yml) | Label source of truth |
| [`sync-labels.sh`](sync-labels.sh) | Applies `labels.yml` to the repo |
| `decisions/` | ADRs, once accepted |
| `docs/` | MkDocs knowledge layer (prd-02) |
| `deploy/` | Host configuration and hardening (prd-03) |
| `upstream-intel/` | Upstream tracking tooling (prd-01) |

The last four arrive with the PRDs that create them.

---

## The one rule

> **Stable knowledge belongs in a document. Active work becomes a GitHub issue.**

No `TODO` comments, no `PLANNED.md`, no roadmap files. If it isn't true yet, it's an
issue. If it is true, it's documentation. A decision still being argued is an ADR issue;
once made it becomes `decisions/ADR-XXXX-slug.md` and the issue closes.
