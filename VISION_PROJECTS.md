# 🐝 Buzz Projects — A Nostr-Native Forge

> Someone pushes a fix. Buzz creates a channel for the branch. The CI agent picks up the push, runs the tests, posts results back to the channel. A co-maintainer reviews the diff inline, approves it — a signed event, cryptographic proof. Merge. The workflow runs the integration. The channel archives into a permanent record of why that code exists.
>
> Bug report to merged patch. One place. One search index. One identity system. The branch channel was the pull request, the CI dashboard, and the discussion thread.

This document is the software-forge slice of the broader Buzz platform. [VISION.md](VISION.md) covers the platform. [VISION_SOVEREIGN.md](VISION_SOVEREIGN.md) covers the sovereign relay story — one domain, one relay, one project. This doc zooms in on what it looks like when that relay hosts code. In multi-community Buzz, the same rule is lifted one level up: a project domain or subdomain selects the community first, and repositories, workflows, approvals, Blossom artifacts, and git ref updates under that host are community-local even if an operator runs many communities on shared backend infrastructure.

---

## The Project Model

A project lives on the relay. `myproject.com` in a browser shows the project home. Click a repo and you're at `repoa.myproject.com` — README rendered, file tree navigable, code syntax-highlighted, clone URL at the top. The same URL serves HTML to a browser and git protocol to `git clone`. Content negotiation. One URL, two audiences.

Git transport is standard Smart HTTP — `git clone`, `git push`, nothing special. Your npub signs pushes. Same domain, same auth, same identity as everything else on the relay. The host in the clone/push URL is also the community selector: the same `owner/repo` name may exist in two communities without sharing refs, branch protections, workflow runs, approvals, or repo announcements.

The portable representation is a NIP-34 repo announcement (kind:30617) — standard metadata that any NIP-34 client can discover and render. Buzz extends it with `buzz-` prefixed tags for channel binding and visibility:

```json
{
  "kind": 30617,
  "tags": [
    ["d", "buzz"],
    ["name", "buzz"],
    ["clone", "https://repoa.myproject.com"],
    ["relays", "wss://myproject.com"],
    ["maintainers", "<co-maintainer-npub>"],
    ["buzz-channel", "<channel-uuid>"],
    ["buzz-visibility", "listed"],
    ["buzz-protect", "main", "push-allowed", "<alice-npub>", "<bob-npub>"],
    ["buzz-protect", "main", "require-approval", "2"],
    ["buzz-protect", "main", "no-force-push"]
  ]
}
```

Branch protections live in the same event — `buzz-protect` tags. The relay enforces them at the git transport layer. Only npubs listed in `push-allowed` can push to protected branches. Force pushes are blocked. Merges require the specified number of signed approval events (kind:46011) before the relay accepts the push.

Agents inherit access from their owner via [NIP-OA](docs/nips/NIP-OA.md). The relay checks: does the push carry a valid NIP-OA auth tag, and is the owner pubkey in that tag listed in `push-allowed`? If yes, the push is accepted — the agent's own pubkey doesn't need to be in the list. Add a maintainer, and all their authorized agents can push. Remove the maintainer, and all their agents lose access instantly. Agents without NIP-OA attestation are treated as their own identity and must be listed explicitly.

Standard NIP-34 clients see a normal repo. gitworkshop.dev renders it. ngit-cli works with it. Buzz clients read the `buzz-` tags and wire up the channel and project UI. One event, two audiences, zero custom kinds.

NIP-34 is the metadata and discovery layer. Git remains the transport. The transport is boring. The metadata is portable.

---

## Branches as Channels

A feature branch is a conversation.

When you create a branch, Buzz creates a channel. The branch's patches, review comments, CI results, and merge decision all live in that channel. When the branch merges, the channel archives. The conversation becomes the permanent record of why that code exists.

```
#feat-auth-fix
├── 🧑 alice: "Starting OAuth2 PKCE implementation"
├── 🤖 ci-agent: "Build triggered — commit a1b2c3d"
├── 🤖 ci-agent: "✅ All 47 tests pass (12.3s)"
├── 📎 kind:1617 patch — src/auth/pkce.rs (+120 lines)
├── 🧑 bob: "One nit on error handling line 45"
├── 📎 kind:1617 patch v2 — addressed review
├── 🤖 review-agent: "LGTM — error variants match trait spec"
├── ✅ bob: Approval event (kind:46011)
├── 🔀 Merged to main — kind:1631
└── 📦 Channel archived
```

No tab-switching between issue tracker, CI dashboard, chat, and code review. The channel IS the pull request, the CI dashboard, and the discussion thread. One stream. One search index.

---

## The Merge Flow

Push to merge, fully traced. Every step is a signed event.

```
Push          CI              Review          Merge
  │            │                │               │
  │ kind:30618 │                │               │
  │ (ref update)               │               │
  │───────────►│                │               │
  │            │ Workflow       │               │
  │            │ triggers       │               │
  │            │                │               │
  │            │ Build ✅       │               │
  │            │ Test ✅        │               │
  │            │ Lint ✅        │               │
  │            │                │               │
  │            │ kind:1630 ────►│               │
  │            │ (CI passed)    │               │
  │            │                │ Review in     │
  │            │                │ branch channel│
  │            │                │               │
  │            │                │ kind:46011    │
  │            │                │ (approved) ──►│
  │            │                │               │
  │            │                │               │ Merge to main
  │            │                │               │ kind:1631
  │            │                │               │
  │            │                │               │ Channel archives
```

The approval event is signed by the maintainer's npub. The merge status references the approval. The audit log chains them together. Cryptographic proof of who approved what.

---

## The Web of Trust

Every contributor — human or agent — has a verifiable identity and a queryable contribution history across every project on the network. Within Buzz, that history is queried through a community boundary: one community can choose to surface reputation from other communities later, but profiles, DMs, memberships, and project records are not implicitly shared across hosts.

A new contributor submits a patch. Before you read the code:

1. **Query their npub** — patches submitted, patches merged, projects contributed to.
2. **Check your trust graph** — have maintainers you trust vouched for this person? Signed approval events are public and queryable.
3. **Assess risk** — fresh npub with no history gets scrutiny. An npub with 50 merged patches across projects you respect gets fast-tracked.

This works because identity is cryptographic and portable. Your npub, your contribution history, and your trust relationships travel with you. No platform owns your reputation.

**For agents**: an agent with a persistent npub and verifiable contribution history is fundamentally different from an anonymous generator. The agent's reputation is on the line with every contribution, across every project it touches. See [NIP-OA](docs/nips/NIP-OA.md) for the owner attestation mechanism that proves which human authorized which agent — independent keys, contained blast radius.

---

## CI and Workflows

Workflows orchestrate. Agents perform the compute. The relay is the message bus, not the build server.

A push to a branch channel triggers the CI workflow. The workflow engine coordinates the steps — build, test, lint. Agents run the actual jobs on their own infrastructure: your server, a cloud function, a laptop. Results post back to the branch channel alongside the conversation.

Workflows live in the repo (`.buzz/workflows/`) or are defined at the project level and inherited by every branch channel automatically — no per-branch configuration, no copy-pasting YAML. Workflow definitions, schedules, webhooks, runs, and approval tokens inherit the project/community selected by the host, so a webhook or cron trigger for one community cannot resolve a same-named workflow in another.

```yaml
name: CI
trigger:
  on: diff_posted
steps:
  - id: build
    action: call_webhook
    url: "https://ci.internal/build"
    body: '{"commit": "{{trigger.commit}}"}'
  - id: test
    action: call_webhook
    url: "https://ci.internal/test"
    if: "steps.build.output.status == 'success'"
  - id: gate
    action: request_approval
    message: "CI passed. Approve merge?"
    if: "steps.test.output.status == 'success'"
```

Every step traced. Every trace a signed event. Change the project CI once and every branch gets it.

---

## Issues, Docs, Releases

### Issues → Forum + NIP-34

Bug reports are NIP-34 kind:1621 events, rendered through Buzz's forum surface. Threaded comments use NIP-22 kind:1111. Labels, assignees, milestones are nostr tags. Design discussions and RFCs use the forum's long-form async surface.

NIP-34 clients can discover and interact with issues. Buzz's forum gives them a home with threading, search, and agent triage.

### Docs → Canvases

Living documents, collaboratively editable by humans and agents via MCP tools. Not static HTML deployed to a CDN — documents that update when the code changes, because the doc writer agent watches ref updates and proposes edits.

### Releases → Agent + Workflow

An agent in `#releases` watches `main`. When a release is needed — triggered by a workflow or by a human posting "ship it" — it assembles the changelog from every merged patch since the last tag, posts a draft. The maintainer approves. The workflow builds artifacts, pushes to content-addressed storage (Blossom/S3), and publishes. Logged, signed, traceable.

---

## Agents as Contributors

Agents are project members with npubs, contribution histories, and reputations. The protocol treats them identically to humans. Visual badges distinguish them in the UI.

| | Human | Agent |
|---|---|---|
| Identity | secp256k1 keypair | secp256k1 keypair |
| Handle | `alice@buzz.dev` | `triage-bot@buzz.dev` |
| Events | Signed with npub | Signed with npub |
| History | On the relay | On the relay |
| Reputation | Earned by contributions | Earned by contributions |

| Role | Watches | Does |
|------|---------|------|
| **Triage** | Issues (kind:1621) | Labels, assigns, detects duplicates, pre-screens |
| **Review** | Patches (kind:1617) | First-pass code review, style checks, dependency audit |
| **Docs** | Ref updates (kind:30618) | Keeps docs in sync after merges |
| **Merge coordinator** | CI results | Runs the merge train, requests human sign-off |
| **Coding agent** | Jobs (kind:43001) | Implements tasks, submits patches for review |

---

## Nostr-Native

Standard kinds as substrate. Custom kinds only where genuinely novel.

| Layer | Standard NIP Kinds | Buzz Custom | Rationale |
|-------|-------------------|---------------|-----------|
| **Git state** | 30617, 30618, 1617, 1618, 1621, 1630-1633 (NIP-34) | — | Interop with ngit, gitworkshop.dev |
| **Comments** | 1111 (NIP-22) | — | Threaded replies everywhere |
| **Channels** | 9000-9022, 39000-39003 (NIP-29) | — | Project workspaces |
| **HTTP auth** | 27235 (NIP-98) | — | Git push authentication |
| **Agent identity** | 0 (NIP-01 profile) | — | Agents are npubs |
| **Artifacts** | 1063 (NIP-94) | — | Build outputs on Blossom/S3 |
| **Workflows** | — | 46001-46012 | No NIP equivalent |
| **Job dispatch** | — | 43001-43006 | Delegation trees |
| **Project binding** | 30617 (NIP-34) | `buzz-` tags | Channel, visibility |
| **Audit** | — | 48001 | Hash-chain tamper-evident log |

If Buzz disappears tomorrow, your repos still work on gitworkshop.dev, your patches still work with ngit-cli, your identities still work on any nostr client. Centralized deployment, decentralized protocol.

---

## Status

| Capability | Status |
|---|---|
| Channels, forums, DMs, canvases | ✅ Ships today |
| Workflow engine (triggers, traces, conditional logic) | ✅ Ships today |
| MCP server + ACP agent harness | ✅ Ships today |
| Blossom media storage (SHA-256, S3) | ✅ Ships today |
| Approval gates | 🚧 Infrastructure exists; executor wiring in progress |
| Project binding (kind:30617 + `buzz-` tags) | 📋 Designed |
| Git hosting (smart HTTP + NIP-34) | ✅ Ships today |
| Merge coordinator | 📋 Designed |
| NIP-34 issues (kind:1621) | 📋 Designed |
| Web-of-trust reputation | 📋 Designed |

The collaboration platform is built, and git hosting ships today — `git clone`/`git push` over smart HTTP with NIP-34 manifests. The forge layer above it is the work ahead — the merge train, project binding, issues, and the reputation system, wired into the surfaces that already exist. See [VISION.md](VISION.md) for the platform and [VISION_SOVEREIGN.md](VISION_SOVEREIGN.md) for the sovereign relay story.

---

*Buzz 🐝 — the forge where identity is the foundation.*
