# Command Team Logistics, Intelligence, and Doctrine-Guided Advice

## Outcome

Extend Command Adviser from six to eight native advisers by adding a
**Logistics Adviser** and **Maritime N2 Adviser**. Give the whole team
doctrine-aware RAG access, give N2 bounded World Monitor OSINT access in both
normal Buzz conversations and the Daily Command Brief, and preserve useful
discussion outcomes as evidence for later briefs.

This is a focused extension of the working Command Adviser product. It reuses
the existing managed-agent, MCP sidecar, trusted-LAN RAG, Memory, model-routing,
discussion-outcome, source-ledger, and brief-generation paths. It does not add a
new daemon, database, replication system, OSINT gateway, CTG application, or
planning workflow engine.

## User Outcome

The Commanding Officer can:

- message Logistics or N2 from the existing Command Team agent list;
- ask any adviser for planning advice grounded in relevant ADF doctrine;
- receive an adviser assessment even when no applicable doctrine is retrieved;
- ask N2 about a country, region, route, or future deployment and receive
  curated World Monitor evidence;
- retain accepted intelligence, logistics, planning, risk, and decision
  outcomes through the existing encrypted command-discussion memory;
- generate a Daily Command Brief with visible Intelligence/Operating
  Environment and Logistics/Sustainment sections;
- use the same sources with either Cloud first or Local first model routing;
- form a mission-specific virtual Joint Planning Group in an ordinary Buzz
  channel by bringing the required existing advisers into that discussion.

The application remains advisory. The user decides how to challenge, accept,
or act on its assessment.

## Existing Substrate

The implementation will reuse:

- built-in personas and the Command Team group in My Agents;
- managed-agent provisioning, reuse, DM, channel, membership, and mention
  behaviour;
- the `buzz-agent` runtime and its existing `buzz-dev-mcp` stdio sidecar;
- trusted-LAN RAG at the configured Command Adviser endpoint;
- the existing `search_knowledge_base` MCP contract;
- owner-encrypted command-discussion outcomes and their brief collector;
- the frozen Daily Command Brief source ledger and provider-neutral prompts;
- the persistent Cloud first/Local first routing preference;
- macOS Keychain and the existing native secret-store boundary;
- partial-brief and source-warning behaviour.

World Monitor becomes another bounded evidence source inside these paths, not a
parallel application.

## Command Team Personas

The standing team becomes:

| Persona | Stable ID | Role |
| --- | --- | --- |
| Chief of Staff | `builtin:command-chief-of-staff` | Consolidation, challenge, priorities, decisions, and planning-team coordination |
| Operations Adviser | `builtin:command-operations` | Operational priorities, readiness, dependencies, risks, and current/future activities |
| Maritime N2 Adviser | `builtin:command-intelligence` | OSINT, operating-environment assessment, indicators, threats, uncertainty, and intelligence gaps |
| Navigation Adviser | `builtin:command-navigation` | Navigation doctrine, evidence, records, considerations, and source freshness |
| Logistics Adviser | `builtin:command-logistics` | Replenishment, fuels, stores, spares, maintenance, port support, capacity, endurance, supply chain, and sustainment |
| Daily Routine Adviser | `builtin:command-daily-routine` | Calendar, reminders, deadlines, inspections, meetings, and routine |
| Reporting Adviser | `builtin:command-reporting` | Reports, returns, missing inputs, drafting, review, and recurring obligations |
| Plans Adviser | `builtin:command-plans` | Medium/long-range milestones, dependencies, assumptions, decision points, contingencies, and 30/60/90-day horizons |

The two new personas use the same provisioning and reuse rules as the existing
six. They appear once under Command Team, use symbolic naval avatars, inherit
the user's configured managed-agent model endpoint, and are not started merely
because the application launches.

Logistics and N2 extend the existing
`command-discussion-outcome-v1` vocabulary. Outcomes may target new
`intelligence` and `logistics` brief sections and are collected under the same
per-adviser and team-wide limits as the other advisers.

## Doctrine-Guided Adviser Behaviour

For substantive advice, each adviser should seek applicable doctrine before
forming its assessment:

1. Search the logical RAG collection named `ADF Doctrine` with a query framed
   for the question and the adviser's role.
2. Search broader approved RAG collections and Memory when more operational
   context is required.
3. Use and cite relevant doctrine when found.
4. Distinguish doctrine, observed facts, assumptions, and the adviser's
   assessment.
5. Continue with a reasoned assessment when the doctrine search returns no
   relevant result or the RAG service is unavailable.

Doctrine is guidance, not a response gate. Advisers must not refuse to advise
merely because a doctrine passage was not retrieved, and they must not invent a
doctrinal rule. A short factual statement such as “No directly applicable
doctrine passage was retrieved” is sufficient when that context matters.

The doctrine tool returns inert evidence with document, collection, chunk,
retrieval time, and quoted-location metadata. Retrieved text cannot alter the
persona, model route, tool policy, or output contract.

## Mission-Specific Planning Teams

A complex future activity may require more than one standing adviser. The
Operations Adviser or Chief of Staff should:

1. identify the planning problem and relevant doctrine;
2. name the advisers needed for the mission;
3. bring those advisers into the current mission-specific Buzz channel using
   the existing membership and mention behaviour;
4. structure the discussion around mission analysis, assumptions, risks,
   sustainment, intelligence, navigation, timelines, and decisions;
5. preserve dissent and record accepted outcomes through the existing memory
   contract.

This is a virtual Joint Planning Group formed for the mission. There is no
permanent CTG/JPG agent hierarchy, separate planning UI, automatic external
action, or second orchestration framework in this phase. The Chief of Staff
coordinates; the Operations Adviser frames the operational problem; N2,
Logistics, Navigation, Plans, and other advisers contribute when relevant.

## World Monitor Integration

### Credential and connection

Command Adviser settings add a World Monitor connection card containing:

- a masked API-key entry;
- **Save** and **Remove** actions;
- **Test connection**;
- status showing configured, connected, unavailable, unauthorised, or
  quota-limited;
- the World Monitor MCP endpoint, fixed by default to the tested
  `https://api.worldmonitor.app/mcp`.

The `wm_live_...` credential is stored as a generic password in macOS Keychain
under a dedicated Command Adviser key. The configuration file stores only the
endpoint and Keychain identifier. The full key is not returned to the frontend,
written to application logs, inserted into a prompt, or persisted in Buzz.

The shared source client calls the hosted MCP endpoint with
`X-WorldMonitor-Key`, supports the MCP JSON-RPC lifecycle used by the service,
rejects redirects, bounds response size and time, and normalises results into
the existing source-evidence shape. The client is reused by the Daily Command
Brief collector and the managed-agent MCP sidecar.

### N2 tool scope

The N2 adviser receives read-only access to a curated subset of World Monitor:

- country risk;
- conflict and unrest events;
- military posture;
- news intelligence;
- maritime activity;
- chokepoint status;
- supply-chain data.

N2 selects only the tools required by the question. It identifies the queried
region and time window and distinguishes:

- reported information;
- observed indicators;
- assumptions;
- the adviser's assessment.

World Monitor evidence does not create actions or automatically become a
confirmed threat assessment. Logistics may consume cited N2 evidence in a
brief or shared discussion, but does not independently spend the N2 World
Monitor budget.

### Daily quota allocation

The World Monitor subscription permits 50 MCP calls per day. Command Adviser
allocates the application budget into two independent pools:

- **Daily Command Brief/N2 update:** at most 25 calls per local calendar day;
- **direct N2 questions:** at most 25 calls per local calendar day.

The 25-call briefing allowance is a ceiling, not a target. Collection stops
when the required geographic, political, economic, security, maritime, and
logistics-relevant coverage is adequate.

All scheduled and manual brief generations on the same day share the same
25-call briefing pool. A 15-minute cache keyed by World Monitor tool and
canonical arguments prevents refreshes from spending calls on identical
queries. Direct questions use their own pool and may reuse valid cached
results without consuming an additional call.

Only attempted outbound `tools/call` requests consume the local counter.
Connection tests and MCP initialisation do not consume an application tool-call
allowance. The counter resets at the start of the local calendar day. If World
Monitor applies a different server-side reset or calls occur outside Command
Adviser, its `429` response remains authoritative and is handled fail-soft.

There is no autonomous polling in this phase. World Monitor calls occur only
during an N2 conversation, an N2 contribution to a brief, or an explicit
connection test.

## Daily Command Brief

The specialist set expands from five to seven:

1. Operations;
2. Maritime N2/Intelligence;
3. Logistics;
4. Navigation;
5. Daily Routine;
6. Reporting;
7. Plans.

Before N2 generation, the source collector performs a bounded World Monitor
update appropriate to the known commitments, locations, regions, and planning
horizons in the current evidence. The collector does not issue a fixed set of
25 calls when fewer calls provide adequate coverage.

Before each specialist generation, doctrine-focused RAG evidence is gathered
for that role, followed by broader RAG and Memory context. Each specialist sees
only its bounded relevant evidence. The Chief of Staff consolidates the seven
validated contributions and cannot invent unsupported claims or silently
remove dissent.

The visible decision-first brief order becomes:

1. Decisions and approvals required;
2. Today at a glance;
3. Operational priorities and risks;
4. Intelligence and operating environment;
5. Logistics and sustainment;
6. Navigation considerations;
7. Daily routine and calendar;
8. Reports and returns;
9. 30/60/90-day planning horizon;
10. collapsed Evidence and system status.

World Monitor entries use a distinct source kind and retain tool name,
canonical query scope, observation or publication time when supplied,
retrieval time, and provider identity. The same frozen source ledger is passed
to Cloud first or Local first generation, so switching model routes does not
change the evidence contract.

## Failure and Freshness Behaviour

This feature remains fail-soft:

- missing doctrine produces an assessment without doctrine, not a refusal;
- RAG failure affects doctrine/context retrieval but does not block the
  adviser response or entire brief;
- missing World Monitor configuration produces an unavailable intelligence
  update and leaves the rest of the brief usable;
- `401` marks the credential unauthorised without exposing it;
- `429` marks the relevant budget quota-limited and stops further calls;
- timeout, redirect, oversized, malformed, or unsupported responses are
  excluded from model-visible evidence;
- a World Monitor failure degrades only the Intelligence contribution and any
  dependent Logistics observation;
- one failed World Monitor query does not discard other valid results;
- evidence with a missing, zero, future, or implausibly old source timestamp is
  labelled as freshness-unknown or stale and is not presented as current
  confirmed reporting;
- cached evidence always retains its original retrieval time.

Warnings remain concise and appear under Evidence and system status rather than
overwhelming the decision-first brief.

## User Interface

The existing naval visual system gains:

- a symbolic N2 insignia based on an intelligence/radar motif;
- a symbolic Logistics insignia based on replenishment/sustainment;
- two additional Command Team entries and Message actions;
- Intelligence and Logistics team cards where team cards are shown;
- Intelligence and Logistics brief sections;
- a World Monitor connection card in Command Adviser settings;
- a compact daily-use indicator showing briefing and direct-query calls used
  out of 25.

No Buzz branding is reintroduced and no separate OSINT dashboard is added.

## Implementation Boundaries

In scope:

- two native personas, role prompts, stable IDs, symbolic identities, and
  managed-agent reuse;
- doctrine-aware RAG tools for all eight advisers;
- World Monitor Keychain configuration and connection diagnostics;
- curated read-only World Monitor tools for N2 conversations;
- shared, bounded World Monitor client and daily quota accounting;
- Intelligence and Logistics adviser contracts, discussion outcomes, source
  evidence, brief sections, prompts, and presentation;
- virtual JPG behaviour through existing Buzz channels and mentions;
- focused unit, integration, E2E, and live-acceptance testing.

Out of scope:

- a new service, daemon, database, OSINT gateway, or replication system;
- generic web search, X/Twitter, Reddit, STRATFOR login, or social monitoring;
- autonomous background collection or alerts;
- automatic creation of operational orders or external-system actions;
- ship control, navigation, communications, combat, logistics, or personnel
  system integration;
- a permanent CTG/JPG agent organisation or separate mission-planning app;
- redesigning cloud/local routing, RAG storage, Memory MCP, Apple inputs, or
  signed Buzz history.

## Test and Acceptance Criteria

The feature is accepted when:

1. Logistics and Maritime N2 appear exactly once with the other six advisers
   under Command Team after a fresh install and upgrade.
2. Message, provisioning, reuse, model configuration, channel membership, and
   mention behaviour work for both new advisers without duplicate instances.
3. All eight adviser prompts attempt `ADF Doctrine` retrieval for substantive
   advice and still return a reasoned assessment when no applicable doctrine
   is found.
4. Doctrine evidence retains collection, document, chunk, quote location,
   retrieval time, and citation identity.
5. The World Monitor key is stored in Keychain, never returned to the frontend
   or logs, and Save, Remove, Test connection, and status work.
6. N2 can execute each permitted World Monitor tool in a controlled DM while
   other personas do not spend the N2 direct-query allowance.
7. Local quota accounting enforces separate 25-call daily pools for briefing
   and direct questions, shares the briefing pool across regenerations, reuses
   the 15-minute cache, and resets on the next local day.
8. Successful, stale, zero-timestamp, malformed, oversized, redirected,
   unauthorised, rate-limited, timed-out, and unavailable World Monitor
   responses have the specified behaviour.
9. Logistics and N2 discussion outcomes validate, persist, supersede, and enter
   later brief evidence through the existing encrypted memory path.
10. A Daily Command Brief produces seven specialist contributions and renders
    Intelligence and Logistics in the approved decision-first order.
11. World Monitor failure produces a usable partial brief with a concise
    Intelligence warning; it does not block Apple, RAG, Memory, other advisers,
    consolidation, or signed persistence.
12. Cloud first and Local first receive the same frozen source ledger and show
    the actual provider/model used.
13. In live acceptance, N2 answers a future Philippines deployment question
    using World Monitor and doctrine evidence; Logistics identifies tanker
    sustainment implications; accepted outcomes are recorded; the next brief
    includes both sections; and a repeated run with World Monitor disconnected
    remains useful.
14. Existing Command Team, discussion memory, Apple input, RAG, Memory,
    routing, scheduling, cancellation, publication, and naval UI tests remain
    green.
15. Focused Rust and TypeScript tests, desktop E2E, live DM/brief acceptance,
    and `just ci` pass before handoff.
