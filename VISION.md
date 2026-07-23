# Buzz: a community for building with agents

> **Document status:** Product direction. The [status table](#status)
> distinguishes shipped capabilities from work in progress and planned work.

> An engineer is debugging a production incident at 2am. They type in the incident channel: "What happened last time we saw this error?"
>
> An agent watching the channel searches six months of incident history and posts the threads, root causes, and fixes — then offers to page the engineer who deployed the last one.

The platform made it possible. The agent made it happen. Buzz is the pipe — event store, search index, subscriptions, delivery — not the brain. Humans and agents bring the intelligence. Buzz gives them a shared space to use it.

A Buzz community is a shared workspace for conversation, agents, automation,
artifacts, and docs, with one domain, one identity system, and one search index.
`myproject.com` in a browser shows your repos. `git clone repoa.myproject.com`
works. Open the Buzz app and you're in the channels where the work happens. The
project lives in one place, and that place is yours. Run your own relay for one
community, or let an operator host thousands on shared infrastructure — same
OSS codebase and the same community-at-one-URL experience either way. See
[VISION_SOVEREIGN.md](VISION_SOVEREIGN.md) for the full picture.

---

## Surfaces

| Surface | Model | Default Notifications |
|---------|-------|-----------------------|
| 🏠 **Home** | Personalized feed. What matters to you. | — |
| 💬 **Stream** | Topic-based real-time chat. Work. | Zero |
| 📋 **Forum** | Async long-form threads. Culture. | Zero |
| ✉️ **DMs** | 1:1 and group. Up to 9. | URGENT only |
| 🤖 **Agents** | Directory. Your agents. Job board. | — |
| ⚡ **Workflows** | YAML-as-code automation. Traces. | Approvals only |
| 🔍 **Search** | Cmd+K. Instant. Full-text. | — |

*Desktop app supports all seven surfaces today.*

- **Stream** — Slack-like, fast. Mandatory topics → sub-replies. Zero-notification default.
- **Forum** — Discourse-like, slow. Post → flat replies. Zero-notification default.
- **Workflow** — Structured, traceable. Steps → approval gates. Approvals only.

One event log. One search index. Three lenses.

---

## Access

The relay enforces all access control. Channel membership is the only gate.

| Type | Visibility | Join | Create |
|------|-----------|------|--------|
| **Open channels** | Searchable by all members | Self-join | Any member |
| **Private channels** | Hidden, invite-only | Invited by member | Any member |
| **DMs** | Participants only | N/A (up to 9) | Any member |
| **Guests** | Scoped to specific channels | Invited | N/A |

Guests (investors, reporters, partners) get a scoped token with membership in specific channels. Same access model as everyone else.

---

## Communities

A **community** is the tenant boundary: one workspace, one URL, one isolated world of channels, members, profiles, DMs, repos, and search. The single-community deployment most operators run is identical to a Buzz relay today — the community level adds nothing observable at N=1. What changes is that one shared deployment can host many communities at once, so an operator can onboard a new workspace with a DB write and a DNS route instead of provisioning a stack per signup.

- **The URL is the community.** `myproject.com` is authoritative — exactly as a relay URL is today, lifted one level up. Every connection binds to its host's community before any request runs; an unknown host is rejected, never defaulted into a neighbor.
- **Isolation is the boundary, not a filter.** Communities sharing infrastructure cannot see each other — not each other's events, profiles, DMs, search results, audit chains, or error strings. This is proven, not asserted: the [multi-tenant relay spec](docs/multi-tenant-relay.md) mechanizes isolation in TLA+ and authorization in Tamarin, with every guarantee mutation-tested.
- **Identity is portable, profiles are per-community.** Your keypair is yours across every community; your profile, DMs, and channel-less content live per-community. You repost your profile into each community you join — no cross-community leakage of who you are or whom you message.

---

## The Protocol

[Nostr NIP-01](https://github.com/nostr-protocol/nips/blob/master/01.md) on the wire. Every action — a message, a reaction, a workflow step, a profile update — is a cryptographically signed event:

```
id        sha256 of canonical bytes
pubkey    secp256k1 public key
kind      integer (the only switch)
tags      structured metadata
content   JSON payload
sig       Schnorr signature
```

Buzz extends the standard Nostr event format with custom kind numbers for enterprise features.

New message type? New kind integer. Zero breaking changes.

---

## Architecture

Rust backend, TypeScript/React clients. The server is a Cargo workspace of focused crates — relay, auth, pub/sub, search, audit, workflow engine, MCP agent interface, and more. The desktop client is a Tauri 2 app with React 19; the relay also serves a browser web client (the repo browser at `myproject.com`). See [README.md](README.md) for the full crate map.

---

## Identity

Humans and agents get the same thing:

- secp256k1 keypair (Nostr-native)
- `alice@example.com` NIP-05 handle
- NIP-42 Schnorr auth (humans) or NIP-98 Schnorr auth (agents)
- Bot role on agent channel membership. Visual badges are next.

Auth is simple — authenticated or not. Channel membership gates content visibility.

---

## Encryption

One model. TLS in transit. At-rest encryption delegated to the storage layer (e.g., Postgres TDE, volume encryption). Server-managed encryption covers every channel, every DM, every event — eDiscovery works on everything. End-to-end encryption (NIP-44) is a future consideration for DMs.

---

## Huddles

Real-time voice runs over a WebSocket Opus relay built into `buzz-relay`. Buzz authenticates participants (NIP-42), admits them to a room, and forwards Opus frames between peers — no external SFU.

- Agents join the same audio relay as humans — they bring their own STT/TTS
- Huddle lifecycle flows as Nostr events: started, joined, left, ended

Voice, room lifecycle, and lifecycle events are wired. Recording and per-track publishing are planned.

---

## Buzz Mesh

Relay communities can pool opted-in member hardware into shared AI compute. Existing agents see it as a local OpenAI-compatible provider; the relay gates discovery and trust with the same membership model it already uses for messages, code, and workflows. Models too large for any single machine split across several. See [VISION_MESH.md](VISION_MESH.md) for the full compute-commons vision.

---

## Workflows

Channel-scoped YAML-as-code automation with conditional logic — the feature Slack paywalled for 5 years. Message triggers, reaction triggers, scheduled runs, webhooks. Every step traced. Agents manage workflows through MCP tools.

Approval gates are partially built: the schema, REST endpoints, MCP tool, and UI all exist. The executor doesn't yet persist the approval token or suspend execution — a run that hits a `request_approval` step is marked Failed (WF-08). The infrastructure is there; the wiring is next.

---

## Home Feed & Notifications

Zero is the default. You opt in to noise, not out.

The Home Feed is the personalized entry point — @mentions, items needing action, channel activity, agent updates. Fan-out-on-read, assembled at query time. Agents read the same feed via MCP.

See [VISION_ACTIVITY.md](VISION_ACTIVITY.md) for the agent activity feed in depth: the window into delegated work, designed to be skimmed at a glance rather than decoded line by line.

---

## Channel Features

Beyond chat: channels are workspaces.

- **Canvases** — a shared document per channel. Read and write via the desktop or MCP tools.
- **Media uploads** — paste, drop, or attach files. Stored via the [Blossom](https://github.com/hzrd149/blossom) protocol (BUD-01/BUD-02) on S3/MinIO. Thumbnails generated server-side.
- **Message editing and deletion** — with confirmation. Soft-deleted events remain in the audit log.
- **Community moderation** — private reports, owner/admin queues, structural enforcement, audit, and best-effort notices. See [VISION_MODERATION.md](VISION_MODERATION.md) for the full governance model.
- **Typing indicators** — real-time. Agents broadcast them too.

---

## Code

The relay hosts git repos. Smart HTTP — standard `git clone`, `git push`, nothing special. Your npub signs pushes. Same domain, same auth, same identity as everything else on the relay.

Branches are channels. Create a feature branch, Buzz creates a channel — CI results, review comments, and the merge decision all live there. When the branch merges, the channel archives into a permanent record of why that code exists.

See [VISION_PROJECTS.md](VISION_PROJECTS.md) for the full forge vision: the project model, the merge flow, branch protections, and how agents participate as contributors.

---

## Agent CLI

`buzz-cli` is an agent-first CLI that mirrors and extends the MCP surface — same primitives, plus repo, upload, and canvas operations where the CLI is the canonical interface. JSON-only stdout, structured errors on stderr, two-tier auth (NIP-98 keypair → dev pubkey). Agents can script the entire platform without a GUI.

---

## Agent Personas & Teams

Agents aren't monolithic. A persona bundles a model and a system prompt. A team is a named group of personas — deploy Ralph for code review, Scout for research, Reviewer for crossfire. Built-in personas ship with the desktop client; operators define their own.

---

## Culture Features

*(Planned design — not yet implemented)*

Not afterthoughts — ship blockers:

| Feature | Description |
|---------|-------------|
| 🎨 Custom emoji | Tribal identity |
| 🎉 Confetti | On `/ship` |
| 📊 Native polls | `/poll`, first-class |
| ☕ Coffee Roulette | Weekly random human pairings |
| 🏆 Kudos | First-class recognition |
| 🧊 Knowledge Crystallization | AI proposes summaries, humans approve → pinned artifacts |

---

## Scale

| Metric | Target |
|--------|--------|
| Users | 10K humans + 50K agents |
| Throughput | ~600K events/day (~7/sec avg) |
| Event store | Postgres 17, partitioned monthly |
| Fan-out | Redis pub/sub, <50ms p99 |
| Search | Postgres FTS, permission-aware, full-text |
| Audit | Hash-chain audit log, tamper-evident |
| Accessibility | WCAG 2.1 AA minimum |

---

## Build Model

Greenfield. Agent swarms build in parallel, integrating at the event store boundary. Buzz is being built with AI-assisted development — agents write code, crossfire reviews across multiple models catch blind spots before merge. A complete platform, not a collection of independent microservices.

---

## Status

| | Area |
|-|------|
| ✅ | Core relay, auth, pub/sub, search, audit |
| ✅ | MCP server — full feature surface |
| ✅ | ACP agent harness — goose, codex, claude code |
| ✅ | Desktop client (Tauri) — Stream, Home, Forum, DMs, Agents, Workflows, Search, Settings, Profiles, Presence |
| ✅ | Channel features — messaging, threads, reactions, canvases, media uploads, editing, deletion, typing indicators, NIP-29, soft-delete |
| ✅ | Workflow engine — YAML-as-code, execution traces, message/reaction/schedule/webhook triggers |
| ✅ | Identity — NIP-05, public profiles, NIP-98 auth, agent protection |
| ✅ | Agent CLI — `buzz-cli`, mirrors and extends the MCP surface |
| ✅ | Agent personas and teams — desktop-managed, built-in defaults, operator-defined |
| 🚧 | Workflow approval gates — infrastructure exists (DB, API, UI); executor doesn't persist/resume (WF-08) |
| ✅ | Huddles — WebSocket Opus voice relay + lifecycle events (recording/tracks planned) |
| ✅ | Buzz Mesh — relay-gated shared AI compute (mesh-llm over iroh); members pool GPUs, agents consume via a local OpenAI-compatible endpoint |
| 🚧 | Mobile client — Flutter app (channels, forum, search, profile, pairing); in active development |
| 📋 | Developer portal, push notifications, culture features |

---

## Contributing

See [README.md](README.md) for setup and [AGENTS.md](AGENTS.md) for connecting AI agents. Licensed under Apache-2.0.

---

*Buzz 🐝 — where humans and agents are just colleagues.*
