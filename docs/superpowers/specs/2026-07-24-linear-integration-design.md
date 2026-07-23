# Technical Design: Agent-Assisted Linear Integration

**Status:** Proposed
**Date:** 2026-07-24
**Proposal:** [block/buzz#2647](https://github.com/block/buzz/issues/2647)
**License:** Apache-2.0

## 1. Summary

Add a community-wide Linear integration that turns a Buzz discussion into a
trackable engineering task while preserving explicit human control.

A Buzz community administrator installs one Linear workspace connection.
Authorized community members can create Linear issues from Buzz discussions,
agents can prepare plans and report progress, and an approved issue can launch
an implementation agent. Linear is the source of truth for engineering-task
state. Buzz remains the source of truth for conversations, signed user intent,
agent activity, and relay authorization.

The integration has three human gates:

1. Review the generated issue before creating it in Linear.
2. Approve the implementation plan before an agent changes code.
3. Review and approve the pull request before merge.

This design adds no automatic merge or deployment path.

## 2. Goals

- Create a reviewed Linear issue from a Buzz thread without retyping the
  discussion.
- Maintain a durable, tenant-scoped link between a Buzz thread and a Linear
  issue.
- Treat Linear as authoritative for planning, approval, implementation, review,
  completion, and cancellation state.
- Allow users to request Linear actions from Buzz through signed events.
- Start an implementation agent only after a verified Linear webhook confirms
  the mapped approval state.
- Let agents submit plans, progress, blockers, and pull-request links without
  receiving Linear credentials.
- Preserve Buzz's host-derived community boundary and relay-orchestrated service
  architecture.
- Recover safely from process restarts, duplicate deliveries, rate limits, and
  ambiguous network failures.
- Keep merge authority with humans and existing GitHub branch protections.

## 3. Non-Goals

The first version does not:

- Replace Linear projects, cycles, teams, views, or issue workflows.
- Support Jira, GitLab, or arbitrary project-management providers.
- Create issues before a human reviews the generated draft.
- Permit an agent to approve its own plan.
- Start coding before Linear confirms approval.
- Merge pull requests or deploy changes.
- Create more than one Linear issue from a Buzz thread.
- Mirror complete agent transcripts or every tool call into Linear.
- Give Linear OAuth credentials to desktop clients or agents.
- Add a general secret-bearing workflow action.

## 4. Existing Architecture Constraints

The implementation must preserve the following Buzz rules:

- `buzz-relay` is the single source of truth and coordinates cross-subsystem
  behavior.
- Service crates remain isolated from one another. A Linear service must not
  call `buzz-db`, `buzz-pubsub`, or the workflow engine directly.
- New user and agent operations are modeled as signed Nostr events unless the
  operation inherently requires HTTP.
- Channel-scoped events use `h` tags.
- Tenant identity comes from the request host. A client-supplied identifier,
  event tag, OAuth state, or webhook path may not override `TenantContext`.
- All new kinds are registered in `buzz-core/src/kind.rs` and synchronized with
  desktop and mobile constants where applicable.
- External side effects must not make event ingestion depend on Linear
  availability.
- Secrets must not enter events, logs, agent environments, workflow
  definitions, snapshots, or desktop persistence.

The existing workflow approval implementation is not reused as the canonical
approval state. Workflow approvals are scoped to a Buzz workflow run, while
this feature explicitly makes Linear authoritative. Buzz approval controls
request a Linear transition and wait for Linear's verified webhook before
starting implementation.

## 5. Key Decisions

### 5.1 Linear is authoritative after issue creation

Buzz owns the reviewed draft until Linear creates the issue. After creation,
Linear owns the issue's engineering lifecycle.

A click on **Approve plan** in Buzz records a signed intent and requests a
Linear status transition. It does not directly start an agent. Agent launch is
allowed only after a verified Linear webhook projects the issue into the
configured Approved state.

### 5.2 One installation per Buzz community

A community administrator installs one Linear workspace connection. All
authorized humans and agents use the relay-managed connection. Individual
desktop clients do not hold Linear credentials and do not need to remain
online for synchronization.

### 5.3 A dedicated Linear service crate

Add `buzz-linear`, a small I/O service crate responsible for:

- OAuth token exchange, refresh, and revocation.
- Linear GraphQL request and response types.
- Issue, comment, attachment, workflow-state, team, and webhook operations.
- Webhook payload parsing and Linear-specific error classification.
- Rate-limit metadata parsing.

The crate does not know about Buzz databases, authorization, Nostr events,
channels, communities, agents, or pub/sub. `buzz-relay` supplies explicit
inputs and coordinates all cross-system work.

### 5.4 Durable outbox for external mutations

Accepting a Buzz command means the intent is durably recorded, not that Linear
has completed the operation. Linear mutations run from a database-backed
outbox. This keeps the event pipeline available during Linear outages and makes
restart recovery deterministic.

### 5.5 Agents interact through Buzz

Agents publish signed Buzz commands. The relay validates the assigned job and
permitted operation before mutating Linear. Agents never receive OAuth access
tokens, refresh tokens, client secrets, webhook secrets, or the integration
encryption key.

## 6. System Architecture

```text
┌─────────────────────────────────────────────────────────────────────┐
│ Buzz desktop / CLI / agent                                          │
│                                                                     │
│ signed create, transition, plan, progress, blocker, and PR events   │
└───────────────────────────────┬─────────────────────────────────────┘
                                │ WebSocket EVENT
                                ▼
┌─────────────────────────────────────────────────────────────────────┐
│ buzz-relay                                                          │
│                                                                     │
│ auth + role checks → event transaction → integration outbox         │
│          │                                │                         │
│          │                                ▼                         │
│          │                        Linear operation worker            │
│          │                                │                         │
│          │                          buzz-linear                      │
│          │                                │ GraphQL/OAuth            │
└──────────┼────────────────────────────────┼─────────────────────────┘
           │                                ▼
           │                         Linear workspace
           │                                │
           │                                │ signed webhook
           ▼                                ▼
     Postgres projection               /hooks/linear
           │                                │
           └──────── verified projection ───┘
                                │
                                ├── relay-signed status event
                                ├── Redis/local fan-out
                                └── agent job start/cancel
```

### 6.1 Component ownership

| Component | Responsibility |
|---|---|
| `buzz-core` | Kind registry, versioned command/result payloads, validation helpers |
| `buzz-linear` | OAuth, GraphQL, webhook models, Linear error classification |
| `buzz-db` | Installations, mappings, links, projections, outbox, delivery deduplication |
| `buzz-relay` | Tenant binding, authorization, transactions, workers, webhooks, event emission, agent lifecycle |
| `buzz-cli` | Agent-safe issue, plan, progress, blocker, and PR commands |
| Desktop | Installation settings, channel mappings, issue review, linked card, approval controls |
| Linear | Canonical engineering-task lifecycle |
| GitHub | Branch protection, PR review, and human merge authority |

## 7. Canonical Lifecycle

```text
Buzz discussion
    │
    ▼
Draft reviewed in Buzz
    │ create command
    ▼
Linear issue created
    │
    ▼
Read-only planning job
    │ plan submitted
    ▼
Linear: Awaiting approval
    │ approval in Linear or Buzz
    ▼
Linear: Approved ── verified webhook ──► implementation job
    │                                      │
    │                                      ▼
    └────────────────────────────── Linear: In progress
                                           │
                                           ▼
                                      Draft PR opened
                                           │
                                           ▼
                                      Linear: Review
                                           │
                                           ▼
                                Linear: Completed or Canceled
```

### 7.1 Status mapping

Linear workflow states are team-specific. An administrator maps semantic Buzz
stages to immutable Linear workflow-state IDs:

- `planning`
- `awaiting_approval`
- `approved`
- `in_progress`
- `review`
- `completed`
- `canceled`

The UI displays current Linear names, but automation compares IDs. Renaming a
state in Linear does not break the integration.

Each mapped team must provide all seven states before agent automation can be
enabled. Issue creation remains available when automation is disabled.

### 7.2 Launch rules

An implementation job starts only when all conditions are true:

1. The installation is active and its organization matches the issue link.
2. The webhook signature and timestamp are valid.
3. The request host resolves to the link's community.
4. The webhook delivery has not already been processed.
5. The projected Linear state changed into the mapped Approved state.
6. A completed planning job and plan reference exist.
7. No implementation job has been created for the current approval generation.
8. The linked channel and repository remain available.

Moving an issue away from Approved before the job is accepted prevents launch.
Moving an active issue into Canceled requests job cancellation. It does not
delete commits, branches, or pull requests.

Reopening or moving an issue back to Approved does not silently launch a new
agent. A second run requires an explicit authorized retry command, which
increments the approval generation.

## 8. Event Protocol

Reserve the currently unused `48200–48299` system/integration subrange.

| Kind | Constant | Author | Purpose |
|---:|---|---|---|
| 48200 | `KIND_LINEAR_ISSUE_CREATE` | Human | Create a reviewed issue from a thread |
| 48201 | `KIND_LINEAR_ISSUE_TRANSITION` | Human | Request an allowed semantic transition |
| 48202 | `KIND_LINEAR_ISSUE_UPDATE` | Human or assigned agent | Submit a plan, milestone, blocker, or PR |
| 48210 | `KIND_LINEAR_ISSUE_LINKED` | Relay | Confirm issue creation and durable link |
| 48211 | `KIND_LINEAR_ISSUE_SYNCED` | Relay | Publish a sanitized canonical projection |
| 48212 | `KIND_LINEAR_OPERATION_FAILED` | Relay | Publish an actionable operation failure |
| 48220 | `KIND_LINEAR_INSTALLATION_STATUS` | Relay | Publish non-secret community connection health |

All payloads contain a required integer `v` field. Version 1 rejects unknown
required fields only when their meaning would affect authorization or external
side effects; otherwise readers ignore unknown fields.

Installation-status events are community-visible, unchannel-scoped metadata.
They contain only connection state, the Linear workspace display name, and
health timestamps. Detailed failures and configuration remain
administrator-only.

### 8.1 Shared tags

Issue command and result events use:

- `["h", "<channel-uuid>"]`
- `["e", "<thread-root-event-id>", "", "root"]`
- `["linear", "<linear-issue-uuid>"]` after creation
- `["linear-id", "<TEAM-123>"]` on relay result events
- `["operation", "<source-event-id>"]` on result events

The relay derives community, requester, agent assignment, and authorization
from trusted context. Those values are never accepted from event content.

### 8.2 Create command

```json
{
  "v": 1,
  "title": "Add agent-assisted Linear workflow",
  "description": "Markdown issue description",
  "team_id": "linear-team-uuid",
  "project_id": "optional-linear-project-uuid",
  "label_ids": ["optional-linear-label-uuid"],
  "priority": 3
}
```

Validation:

- The `h` channel exists inside the host-derived community.
- The authenticated author is a channel member with message-write access.
- The `e` root belongs to the same channel.
- The channel mapping permits the selected team and optional project.
- Title, description, label count, and payload size use explicit limits.
- No existing active link exists for the thread.

The source event ID becomes the operation correlation ID. The Linear
description includes a non-visible correlation marker:

```html
<!-- buzz-operation:<event-id> -->
```

This marker supports reconciliation after an ambiguous timeout and is removed
from text presented in Buzz.

### 8.3 Transition command

```json
{
  "v": 1,
  "transition": "approve"
}
```

Allowed values are:

- `approve`
- `cancel`
- `retry_implementation`

The relay resolves semantic transitions to configured Linear state IDs.
Clients cannot request arbitrary state IDs.

`approve` and `retry_implementation` require the configured approver role.
`cancel` is allowed to the configured approver role and community
administrators. Additional Linear transitions remain native Linear actions.

### 8.4 Update command

```json
{
  "v": 1,
  "type": "plan",
  "body": "Markdown content",
  "pr_url": null
}
```

Allowed `type` values:

- `context`
- `plan`
- `progress`
- `blocker`
- `pr_opened`
- `completion`

Agent-authored updates require an active planning or implementation job linked
to the same issue and channel. A planning agent may submit only `plan` and
`blocker`. An implementation agent may submit `progress`, `blocker`,
`pr_opened`, and `completion`.

PR URLs must match the repository bound to the channel or agent job. The relay
does not accept an arbitrary GitHub repository URL.

### 8.5 Relay result events

Relay-signed result events contain only:

- Linear issue UUID, human-readable identifier, and URL.
- Sanitized title and mapped state.
- Safe assignee display information.
- Operation state and retryability.
- Timestamps and correlation IDs.
- PR URL after repository validation.

Raw webhook bodies, Linear comments, OAuth data, private project metadata, and
unrelated issue fields are not republished.

## 9. Persistence Model

The following schemas show required fields and constraints. Migration naming
and SQL formatting follow the repository's migration conventions.

### 9.1 `linear_oauth_states`

```text
state_hash                BYTEA PRIMARY KEY
community_id              UUID NOT NULL
administrator_pubkey      TEXT NOT NULL
callback_host             TEXT NOT NULL
pkce_ciphertext           BYTEA NOT NULL
pkce_nonce                BYTEA NOT NULL
key_version               INTEGER NOT NULL
expires_at                TIMESTAMPTZ NOT NULL
consumed_at               TIMESTAMPTZ
created_at                TIMESTAMPTZ NOT NULL
```

OAuth state is persisted so a relay restart between authorization and callback
does not invalidate a legitimate installation. Expired and consumed rows are
removed by bounded retention.

### 9.2 `linear_installations`

```text
community_id              UUID PRIMARY KEY
installation_id           UUID UNIQUE NOT NULL
organization_id           TEXT NOT NULL
organization_name         TEXT NOT NULL
oauth_actor_id            TEXT NOT NULL
credentials_ciphertext    BYTEA NOT NULL
credentials_nonce         BYTEA NOT NULL
key_version               INTEGER NOT NULL
access_token_expires_at   TIMESTAMPTZ NOT NULL
scopes                    TEXT[] NOT NULL
status                    TEXT NOT NULL
last_refresh_at           TIMESTAMPTZ
last_webhook_at           TIMESTAMPTZ
disabled_reason           TEXT
created_by_pubkey         TEXT NOT NULL
created_at                TIMESTAMPTZ NOT NULL
updated_at                TIMESTAMPTZ NOT NULL
UNIQUE (community_id, organization_id)
```

Valid statuses are `active`, `refresh_failed`, `revoked`, and `disabled`.
The encrypted credential envelope contains the access token and rotating
refresh token. One AES-GCM operation and one nonce protect the complete
envelope; nonces are never reused for separate plaintexts.

### 9.3 `linear_channel_mappings`

```text
community_id              UUID NOT NULL
channel_id                UUID NOT NULL
team_id                   TEXT NOT NULL
project_id                TEXT
default_label_ids         TEXT[] NOT NULL
planning_state_id         TEXT NOT NULL
awaiting_approval_state_id TEXT NOT NULL
approved_state_id         TEXT NOT NULL
in_progress_state_id      TEXT NOT NULL
review_state_id           TEXT NOT NULL
completed_state_id        TEXT NOT NULL
canceled_state_id         TEXT NOT NULL
approver_role             TEXT NOT NULL
automation_enabled        BOOLEAN NOT NULL
created_at                TIMESTAMPTZ NOT NULL
updated_at                TIMESTAMPTZ NOT NULL
PRIMARY KEY (community_id, channel_id)
```

Every lookup includes `community_id`; a channel UUID alone is never a tenant
key.

### 9.4 `linear_issue_links`

```text
community_id              UUID NOT NULL
channel_id                UUID NOT NULL
thread_root_event_id      TEXT NOT NULL
link_status               TEXT NOT NULL
linear_issue_id           TEXT
linear_identifier         TEXT
linear_url                TEXT
linear_team_id            TEXT NOT NULL
linear_state_id           TEXT
linear_state_name         TEXT
linear_updated_at         TIMESTAMPTZ
plan_event_id             TEXT
planning_job_id           UUID
implementation_job_id     UUID
approval_generation       INTEGER NOT NULL DEFAULT 0
pr_url                    TEXT
created_by_pubkey         TEXT NOT NULL
created_at                TIMESTAMPTZ NOT NULL
updated_at                TIMESTAMPTZ NOT NULL
PRIMARY KEY (community_id, thread_root_event_id)
```

Create a partial unique index on `(community_id, linear_issue_id)` where
`linear_issue_id IS NOT NULL`. Valid link statuses are `creating`, `linked`,
`failed`, and `unlinked`. This permits a durable reservation before Linear
returns an issue ID without weakening the final uniqueness constraint.

### 9.5 `integration_outbox`

```text
id                        UUID PRIMARY KEY
community_id              UUID NOT NULL
provider                  TEXT NOT NULL
operation                 TEXT NOT NULL
source_event_id           TEXT NOT NULL
correlation_id            TEXT NOT NULL
actor_pubkey              TEXT NOT NULL
payload                   JSONB NOT NULL
state                     TEXT NOT NULL
attempt_count             INTEGER NOT NULL
next_attempt_at           TIMESTAMPTZ NOT NULL
lease_owner               TEXT
lease_expires_at          TIMESTAMPTZ
external_id               TEXT
last_error_code           TEXT
last_error_summary        TEXT
created_at                TIMESTAMPTZ NOT NULL
updated_at                TIMESTAMPTZ NOT NULL
UNIQUE (community_id, provider, source_event_id, operation)
```

The payload contains no plaintext OAuth credential. Workers claim rows with a
bounded lease so another relay node can recover abandoned work.

### 9.6 `linear_webhook_deliveries`

```text
community_id              UUID NOT NULL
delivery_id               UUID NOT NULL
installation_id           UUID NOT NULL
organization_id           TEXT NOT NULL
event_type                TEXT NOT NULL
entity_id                 TEXT
entity_updated_at         TIMESTAMPTZ
received_at               TIMESTAMPTZ NOT NULL
processed_at              TIMESTAMPTZ
outcome                   TEXT NOT NULL
error_summary             TEXT
PRIMARY KEY (community_id, delivery_id)
```

The table stores delivery metadata, not the complete webhook body.

## 10. Transactions and Idempotency

### 10.1 Command ingestion

For a valid command, one database transaction:

1. Inserts the signed event.
2. Checks or creates the thread link reservation.
3. Inserts one outbox operation using the event ID as idempotency key.
4. Commits.

Fan-out and the external mutation happen after commit. Linear unavailability
does not reject a valid Buzz event.

### 10.2 Issue creation ambiguity

Linear issue creation may succeed even if the relay loses the response.
Therefore the worker does not blindly repeat an ambiguous create operation.

Recovery order:

1. Wait briefly for a matching verified webhook containing the correlation
   marker.
2. Query recently created issues in the selected team for the exact marker.
3. If one match exists, persist the link and complete the operation.
4. If no match exists, retry within the bounded policy.
5. If several matches exist, stop automatically and require administrator
   reconciliation.

The unique thread-link constraint prevents two completed links even if two
relay workers race.

### 10.3 Comments and updates

Linear comments include:

```html
<!-- buzz-event:<event-id> -->
```

Webhook reconciliation treats this marker as the external idempotency key.
Duplicate webhook deliveries and worker retries do not create duplicate Buzz
projection events.

### 10.4 Webhook ordering

The relay compares the webhook entity's Linear update timestamp with
`linear_issue_links.linear_updated_at`. Older updates are recorded with outcome
`stale` and ignored. Equal updates are idempotent.

## 11. OAuth and Credential Security

### 11.1 Authorization flow

Use Linear OAuth authorization-code flow with PKCE and `actor=app`.

1. A community administrator sends a NIP-98-authenticated authorization
   request.
2. The relay creates random state and a PKCE verifier.
3. The relay stores only a hash of state, bound to:
   - community
   - administrator pubkey
   - canonical callback host
   - PKCE verifier
   - ten-minute expiry
4. The browser opens Linear authorization.
5. Linear redirects to the community's callback host.
6. The callback resolves `TenantContext` from the host and verifies the
   single-use state.
7. The relay exchanges the code, queries the organization and app actor, then
   encrypts tokens before persistence.
8. Linear automatically creates the OAuth-app webhook configured for that
   community's canonical host.
9. The state record is consumed whether exchange succeeds or fails.

Initial OAuth scopes are the minimum Linear scopes required for reading teams,
workflow states, issues, and comments and for creating/updating issues,
comments, and attachments. Assignable and mentionable agent scopes are not
requested in the first release because Buzz, not Linear, launches the coding
agent.

### 11.2 OAuth application configuration

Linear OAuth application callback and webhook URLs are static application
configuration. To preserve Buzz's host-derived tenant boundary, each Buzz
community uses an OAuth application configuration whose callback and webhook
URLs point to that community's canonical host:

```text
https://<community-host>/integrations/linear/callback
https://<community-host>/hooks/linear
```

For the default self-hosted mode this is one OAuth application for the one
community. A multi-community operator configures one Linear OAuth application
per community host. A shared deployment-wide webhook that chooses a tenant
from `organizationId` is explicitly forbidden because it would bypass
host-derived `TenantContext`.

The operator supplies, through environment or an external secret manager:

- OAuth client ID.
- OAuth client secret.
- OAuth webhook signing secret.
- Canonical community URL.
- Versioned integration-encryption keys.

The client secret, webhook signing secret, and encryption keys are process
configuration and are never persisted in Postgres. The desktop integration
card reports configuration availability without exposing values.

The OAuth application webhook subscribes to Issue, Comment, and OAuth
authorization/revocation events. Linear creates the organization webhook when
the administrator authorizes the application.

### 11.3 Encryption at rest

Use AES-256-GCM, following the existing cryptographic dependency precedent in
`buzz-push-gateway`.

The operator supplies a versioned 32-byte integration key. Associated data
binds ciphertext to:

```text
community_id || installation_id || organization_id || "linear_credentials" || key_version
```

The relay refuses to enable Linear integration when the configured key is
missing or invalid, but the rest of Buzz starts normally.

Key rotation supports decrypting with old versions and writing with the current
version. A background rotation rewrites token ciphertext without changing
OAuth authorization.

Decrypted token values are zeroized after request construction where practical
and never implement `Debug` or serialization.

The PKCE verifier uses a separate encryption operation and nonce, with
associated data bound to the OAuth state hash, community, callback host, and
key version.

### 11.4 Token refresh

- Refresh before the access token reaches its expiry safety window.
- Serialize refresh per installation with a database lease.
- Persist the new access and refresh tokens atomically because Linear rotates
  refresh tokens.
- A transient refresh error retries through the outbox policy.
- `invalid_grant`, deauthorization, or repeated permanent failure disables the
  installation and blocks new mutations and agent launches.

## 12. HTTP Surface

HTTP is used only where an external protocol requires it.

### 12.1 `POST /integrations/linear/authorize`

- NIP-98 authenticated.
- Community-administrator only.
- Returns an authorization URL and expiry.
- Never returns client secret, PKCE verifier, or stored OAuth data.

### 12.2 `GET /integrations/linear/callback`

- Host-derived tenant binding.
- Single-use state verification.
- OAuth code exchange.
- Returns a minimal success/failure page that directs the user back to Buzz.
- Uses no client-provided community identifier.

### 12.3 `POST /hooks/linear`

Processing order:

1. Resolve `TenantContext` from the host.
2. Read the bounded raw body without parsing.
3. Load the community's installation and configured webhook signing secret.
4. Verify `Linear-Signature` with HMAC-SHA256 in constant time.
5. Verify the webhook timestamp is within 60 seconds.
6. Parse JSON.
7. Verify `organizationId` matches the installation.
8. Insert the delivery ID or return success for a duplicate.
9. Apply the projection transaction.
10. Return `200` only after durable acceptance.

Webhook requests use explicit body-size and processing-time limits. They bypass
normal NIP-98 authentication only after passing Linear signature verification.

Disconnect and mapping operations remain signed Nostr commands rather than new
HTTP endpoints.

## 13. Authorization

| Action | Default permission |
|---|---|
| Install, reconnect, disconnect | Community administrator |
| Configure channel/team/status mapping | Community administrator |
| Inspect detailed integration health | Community administrator |
| Create issue from thread | Channel member with message-write access |
| Add human context | Channel member with message-write access |
| Approve plan | Configured approver role; defaults to channel admin/moderator |
| Cancel implementation | Configured approver role or community administrator |
| Retry implementation | Configured approver role |
| Submit plan | Agent assigned to the linked planning job |
| Report implementation progress | Agent assigned to the linked implementation job |
| View linked issue card | Channel member |

An agent cannot:

- Install or disconnect Linear.
- Change channel mappings.
- Approve its own plan.
- Transition an unrelated issue.
- Report against a job it does not own.
- Start another agent.
- Access OAuth or webhook credentials.

Authorization is rechecked when the outbox worker executes. Revoked membership
or roles prevent a delayed operation from using stale authority.

## 14. Agent Integration

### 14.1 Planning job

Issue creation schedules a planning-only `KIND_JOB_REQUEST` after the durable
link exists.

The job receives:

- Linear identifier and sanitized issue snapshot.
- Buzz thread context the agent is authorized to read.
- Repository reference associated with the channel.
- Explicit `mode: "plan"`.
- A policy denying file edits, commits, pushes, and PR creation.

The planning agent submits a structured plan through `buzz-cli`. The relay
posts the plan to Linear and requests the mapped Awaiting approval state.

### 14.2 Implementation job

A verified Approved transition creates an implementation job with:

- The approved Linear issue and plan.
- Current Buzz thread context.
- Repository and base-branch information.
- Existing `AGENTS.md` and repository guidance.
- A correlation identifier for progress reporting.

Job creation and `linear_issue_links.implementation_job_id` update occur in one
transaction. This is the exactly-once launch boundary.

### 14.3 Agent CLI

Add agent-facing commands first in `buzz-cli`, consistent with repository
guidance:

```text
buzz linear issue show --channel <uuid> --event <thread-root>
buzz linear plan submit --channel <uuid> --event <thread-root> --file <path>
buzz linear progress report --channel <uuid> --event <thread-root> --body <text>
buzz linear blocker report --channel <uuid> --event <thread-root> --body <text>
buzz linear pr link --channel <uuid> --event <thread-root> --url <github-pr>
```

The CLI signs events through the existing relay credentials. It does not call
Linear directly.

### 14.4 Pull requests

The implementation agent includes the Linear identifier in the branch name,
commit, or pull-request title according to the workspace's Linear GitHub
integration rules.

On `pr link`:

1. The relay verifies the URL belongs to the repository bound to the job.
2. Buzz records and displays the URL.
3. The relay adds a Linear attachment when needed.
4. The relay requests the mapped Review state.

Linear's existing GitHub integration remains responsible for native PR linking
and any configured status automation. Buzz does not merge the PR.

## 15. Desktop Experience

### 15.1 Community settings

Add a Linear card showing:

- Connected workspace.
- OAuth actor.
- Connection status.
- Last successful token refresh.
- Last verified webhook.
- Reconnect and disconnect actions.
- Team and workflow-state mapping.

No token or secret values are displayed.

### 15.2 Channel settings

Administrators configure:

- Linear team.
- Optional project.
- Default labels.
- Seven semantic state mappings.
- Approver role.
- Whether agent automation is enabled.

The UI validates that mapped states belong to the selected team and that all
required automation states are distinct.

### 15.3 Issue creation

A thread action **Create Linear issue** opens a review dialog containing:

- Agent-generated title.
- Problem statement and desired outcome.
- Scope and non-goals.
- Acceptance criteria.
- Open questions.
- Team, project, labels, and priority.

The user may edit every issue field. Nothing is sent until confirmation.

After confirmation, the dialog shows `Pending` until a linked or failed result
event arrives. Closing the dialog does not cancel the durable operation.

### 15.4 Linked issue card

The thread displays:

- Identifier and title.
- Canonical Linear state.
- Assignee.
- Planning or implementation activity.
- Last synchronization time.
- PR link when available.
- **Open in Linear**.
- Approval, cancellation, or retry controls when authorized.
- Reconnect guidance when the installation is unhealthy.

Approval controls appear only when the projected Linear state is Awaiting
approval and the current member has the configured role.

## 16. Failure Handling

### 16.1 Retryable failures

Retry with bounded exponential backoff and jitter:

- Connection and DNS failures.
- Timeouts before a definitive response.
- Linear rate-limit errors.
- HTTP `429`.
- Linear and upstream `5xx` responses.

Honor Linear rate-limit reset metadata where available. Limit attempts and
total operation age; expired operations move to `needs_attention`.

### 16.2 Permanent failures

Do not retry automatically:

- Invalid issue data.
- Missing team, project, state, or label.
- Lost Linear permission.
- Revoked OAuth authorization.
- Cross-community or organization mismatch.
- Invalid PR repository.
- Unauthorized Buzz actor.

Emit a sanitized `KIND_LINEAR_OPERATION_FAILED` result with an administrator
action when appropriate.

### 16.3 Installation failure

When the connection becomes invalid:

- Preserve issue links and projections.
- Disable new mutations and agent launches.
- Continue displaying the last known state as stale.
- Notify community administrators.
- Resume pending operations only after successful reconnection and
  reauthorization checks.

### 16.4 Cancellation

Canceled canonical state:

- Prevents an unstarted job.
- Requests cancellation of an active job.
- Stops future agent-authored Linear mutations except a final cancellation
  acknowledgement.
- Does not delete Git history or close a PR automatically.

## 17. Observability and Audit

### 17.1 Metrics

- Linear API requests by operation and outcome.
- OAuth exchange and refresh outcomes.
- Webhook signature and timestamp failures.
- Duplicate and stale webhook deliveries.
- Outbox depth, oldest age, retries, and terminal failures.
- Issue-creation latency.
- Approval-to-agent-start latency.
- Agent starts prevented by authorization, stale state, or installation health.

Metric labels use bounded values and never include issue title, description,
token, event content, or user-provided text.

### 17.2 Audit records

Record:

- Community.
- Buzz actor pubkey.
- Channel and thread.
- Linear issue identifier.
- Requested operation.
- Authorization decision.
- Outcome.
- Correlation ID.
- Timestamp.

Never record:

- OAuth access or refresh tokens.
- Client secret or encryption key.
- Authorization code or PKCE verifier.
- Raw webhook body or signature.
- Complete issue descriptions.
- Agent prompts or transcripts.

## 18. Testing Strategy

### 18.1 Unit tests

- OAuth state generation, binding, expiry, and single use.
- AES-GCM encrypt/decrypt and associated-data mismatch.
- Refresh-token rotation.
- GraphQL request and response parsing.
- Linear error and retry classification.
- Status mapping.
- Command payload and tag validation.
- HMAC verification and constant-time comparison.
- Webhook timestamp validation.
- Role and agent-job authorization.
- PR repository validation.

### 18.2 Database tests

- Community-scoped installation, mapping, and link queries.
- One issue per thread.
- One thread per Linear issue.
- Outbox idempotency and lease recovery.
- Duplicate webhook delivery handling.
- Stale webhook rejection.
- Exactly-once implementation job creation.
- Token ciphertext never equals plaintext.
- Cross-community rows with identical external IDs remain isolated.

### 18.3 Relay integration tests

Use a controllable fake Linear server to test:

- OAuth success, denial, invalid state, callback-host mismatch, and expiry.
- Token refresh and refresh races.
- Issue creation and link projection.
- Lost mutation response followed by webhook reconciliation.
- Rate limiting and retry scheduling.
- Duplicate and out-of-order webhooks.
- Revocation and reconnection.
- Planning submission and approval.
- Cancellation before and during implementation.
- Relay restart during every external operation state.

### 18.4 Security tests

- Host A cannot use Host B's OAuth state or installation path.
- Organization A cannot update Organization B's issue projection.
- Invalid, truncated, stale, and replayed webhook signatures fail closed.
- OAuth values never appear in events, logs, errors, snapshots, or agent
  environments.
- Webhook and GraphQL body-size limits are enforced.
- GraphQL destination is fixed to Linear and cannot be turned into an SSRF
  target.
- Delayed outbox work rechecks current actor authority.

### 18.5 Desktop tests

- Connection, reconnect, disconnect, and unhealthy states.
- Team and state mapping validation.
- Issue review and editing.
- Pending, linked, failed, and stale cards.
- Approval controls by role and canonical state.
- Agent progress, blocker, cancellation, and PR states.
- Community switching resets Linear projections and pending UI state.

### 18.6 End-to-end acceptance flow

```text
discussion
→ reviewed issue
→ Linear issue
→ read-only plan
→ awaiting approval
→ human approval
→ implementation agent
→ progress
→ draft PR link
→ review
→ human merge
→ Linear completion
```

The test must also prove that omitting approval never starts an implementation
job.

## 19. Delivery Plan

### Phase 1: Connection and issue creation

- `buzz-linear` OAuth and GraphQL foundation.
- Installation encryption and administration.
- Channel/team mapping.
- Reviewed issue creation.
- Durable issue links.
- Linked issue card.

### Phase 2: Bidirectional projection

- Signed webhook endpoint.
- Delivery deduplication and ordering.
- Status projection.
- Human context and milestone forwarding.
- Health and retry UI.

### Phase 3: Planning and approval

- Read-only planning jobs.
- Plan submission.
- Awaiting-approval transition.
- Buzz and Linear approval paths.
- Exactly-once implementation launch.

### Phase 4: Implementation and PR lifecycle

- Progress and blocker reporting.
- Cancellation.
- PR validation and linking.
- Review transition.
- Completion derived from Linear.

Each phase is independently useful. Agent automation is feature-gated until
Phase 3's approval and exactly-once tests pass.

## 20. Rollout and Compatibility

- The integration is disabled by default.
- Relays without Linear configuration behave exactly as before.
- Clients that do not understand the new kinds ignore them.
- New tables are additive and community-scoped.
- Existing workflows and approval events are unchanged.
- Initial rollout should target one test Linear workspace and one Buzz
  community.
- Administrators can disable automation while retaining issue creation and
  synchronization.
- A kill switch prevents new external mutations and agent launches without
  deleting links or projections.

## 21. Licensing and External Service Boundaries

Buzz is licensed under Apache License 2.0. New source files, tests, migrations,
and documentation produced for this integration remain under the repository's
Apache-2.0 license and existing copyright policy.

The design uses Linear's public OAuth, GraphQL, and webhook protocols. It does
not require copying Linear source code or bundling the Linear TypeScript SDK.
The Rust integration can use the repository's existing HTTP and serialization
stack.

Linear trademarks, logos, hosted service terms, and API usage policies remain
separate from the Buzz source license. Product UI should follow Linear's brand
guidelines, and operators are responsible for creating or configuring an OAuth
application and complying with the Linear service terms.

## 22. Alternatives Considered

### Generic workflow actions

Adding `linear_create_issue` and `linear_update_issue` directly to
`buzz-workflow` appears small but gives a generic automation engine access to
workspace credentials and bypasses the intended lifecycle. OAuth refresh,
webhook ordering, deduplication, and canonical status projection are also
larger than a workflow action boundary.

Rejected for the primary integration. A future workflow action may call the
relay's authorized integration commands after the lifecycle is established.

### Desktop-managed credentials

Storing Linear credentials in the Tauri keychain avoids relay-side encryption,
but synchronization and agent operation stop when that desktop is offline.
Other users would observe inconsistent state.

Rejected because it conflicts with a community-wide source of truth.

### Linear's built-in coding agent only

Delegating implementation entirely to Linear reduces Buzz implementation work,
but it prevents Buzz-managed agents from remaining first-class participants
and makes Buzz unable to enforce its own channel, owner, repository, and agent
policies.

Rejected as the only path. A future adapter may delegate approved issues to
Linear's agent while retaining the same canonical lifecycle.

### Buzz-owned status

Maintaining an independent Buzz state machine and periodically copying it to
Linear creates two authorities and unavoidable conflict resolution.

Rejected. Buzz stores a projection; Linear owns engineering state.

## 23. References

- [Feature proposal: block/buzz#2647](https://github.com/block/buzz/issues/2647)
- [Buzz architecture](../../../ARCHITECTURE.md)
- [Buzz contributor guide](../../../CONTRIBUTING.md)
- [Linear OAuth 2.0](https://linear.app/developers/oauth-2-0-authentication)
- [Linear OAuth actor authorization](https://linear.app/developers/oauth-actor-authorization)
- [Linear GraphQL API](https://linear.app/developers/graphql)
- [Linear webhooks](https://linear.app/developers/webhooks)
- [Linear rate limits](https://linear.app/developers/rate-limiting)
- [Linear agent integration guidance](https://linear.app/developers/agents)
- [Apache License 2.0](https://www.apache.org/licenses/LICENSE-2.0)
