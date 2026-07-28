# Index

Catalog of all wiki pages, organized by category.

## Entities

- [Relay](entities/relay) — the Nostr relay server; the single source of truth for a community
- [Community](entities/community) — a workspace; one URL, one isolated tenant
- [Agent](entities/agent) — an AI agent as a first-class team member with its own Nostr keypair
- [Channel](entities/channel) — stream, forum, and DM communication spaces
- [NostrEvent](entities/nostr-event) — the atomic unit of all activity; a cryptographically signed event

## Concepts

- [Architecture](concepts/architecture) — system overview, layers, and design principles
- [EventPipeline](concepts/event-pipeline) — the 12-step lifecycle of an event from receipt to fan-out
- [Authentication](concepts/authentication) — NIP-42 challenge-response and NIP-98 HTTP auth
- [ChannelMembership](concepts/channel-membership) — access control model (membership is the only gate)
- [WorkflowEngine](concepts/workflow-engine) — YAML-as-code channel automation with triggers and actions
- [AuditLog](concepts/audit-log) — SHA-256 hash-chain tamper-evident audit trail
- [BuzzMesh](concepts/buzz-mesh) — inter-relay mesh for pooled AI compute
- [ACP](concepts/acp) — Agent Communication Protocol bridging Nostr events to agent subprocesses
- [GitIntegration](concepts/git-integration) — NIP-34 git hosting, signing, and branch-as-room
- [MediaStorage](concepts/media-storage) — Blossom protocol media uploads on S3/MinIO
- [Search](concepts/search) — Postgres FTS full-text search over events
- [MultiTenancy](concepts/multi-tenancy) — community isolation model and formal verification
- [NostrProtocol](concepts/nostr-protocol) — Nostr wire protocol and Buzz's custom event kinds

## Components

- [buzz-relay](components/buzz-relay) — main WebSocket relay server (Axum)
- [buzz-core](components/buzz-core) — core types, event verification, filter matching (zero I/O)
- [buzz-db](components/buzz-db) — Postgres event store and data access layer
- [buzz-auth](components/buzz-auth) — auth, rate limiting, and scope enforcement
- [buzz-pubsub](components/buzz-pubsub) — Redis pub/sub, presence, typing indicators
- [buzz-search](components/buzz-search) — Postgres FTS search index
- [buzz-audit](components/buzz-audit) — hash-chain audit log
- [buzz-media](components/buzz-media) — Blossom/S3 media storage
- [buzz-workflow](components/buzz-workflow) — YAML workflow engine
- [buzz-acp](components/buzz-acp) — ACP harness for agent subprocess orchestration
- [buzz-agent](components/buzz-agent) — minimal ACP-compliant agent binary
- [buzz-dev-mcp](components/buzz-dev-mcp) — MCP server providing shell and file-edit tools
- [buzz-persona](components/buzz-persona) — agent persona packs (model + system prompt)
- [buzz-cli](components/buzz-cli) — agent-first CLI (JSON in/out)
- [buzz-sdk](components/buzz-sdk) — typed Nostr event builders
- [buzz-admin](components/buzz-admin) — operator CLI for relay administration
- [buzz-ws-client](components/buzz-ws-client) — shared WebSocket client (NIP-42)
- [buzz-test-client](components/buzz-test-client) — integration test harness
- [buzz-conformance](components/buzz-conformance) — multi-tenant conformance tests
- [buzz-push-gateway](components/buzz-push-gateway) — push notification gateway
- [buzz-relay-mesh](components/buzz-relay-mesh) — iroh-based inter-relay mesh transport
- [git-sign-nostr](components/git-sign-nostr) — sign git objects with a Nostr key
- [git-credential-nostr](components/git-credential-nostr) — git credential helper for Nostr auth
- [sprig](components/sprig) — all-in-one harness (ACP + agent + MCP)
- [DesktopClient](components/desktop-client) — Tauri 2 + React 19 native desktop app
- [MobileClient](components/mobile-client) — Flutter mobile app (iOS + Android)
- [WebClient](components/web-client) — browser-based web client (repo browser)
- [AdminWeb](components/admin-web) — admin dashboard

## Operations

- [DevelopmentSetup](operations/development-setup) — local dev environment setup with Docker Compose
- [Deployment](operations/deployment) — production deployment with Helm and Docker Compose
- [Configuration](operations/configuration) — environment variables and community config
- [CLIReference](operations/cli-reference) — just commands and common dev workflows
- [Troubleshooting](operations/troubleshooting) — common issues and fixes
