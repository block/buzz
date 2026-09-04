# Buzz Plugin System

Status: proposed; no plugin runtime ships yet.

This document describes a proposed Desktop plugin system. It is a public
architecture proposal, not a description of current Buzz behavior. The relay,
Desktop routes, Tauri commands, settings panels, and event-kind registry remain
compiled and closed until this proposal is implemented.

## Goals

The system should:

- provide one versioned contract for code review, stewardship, and loop
  management plugins;
- let a trusted distribution add, update, disable, or withdraw a plugin
  without a Buzz release;
- keep open-source Buzz useful without private service URLs, trust roots,
  credentials, plugin IDs, or rollout policy;
- contain ordinary plugin protocol/process failures without corrupting core records;
- keep identity, relay authorization, signing, confirmation, and audit in
  Buzz; and
- expose bounded, versioned metadata that a later marketplace could index.

## Version-1 non-goals

Version 1 does not add a Buzz wire protocol, relay-hosted plugin code, new
relay kinds, a plugin SDK, plugin-defined routes or CLI commands, plugin
dependencies, plugin-to-plugin calls, or mobile/web plugin UI. It does not
load package JavaScript, React, HTML, CSS, images, iframes, or webviews.

It does not support remote MCP runtimes, Streamable HTTP, host-dispatched
plugin HTTP, a host database/key-value/AI/event/log API, inbound webhooks to
Desktop, or guaranteed work while Desktop is closed. It does not claim to
contain malicious native code running as the same operating-system user, or
provide portable hard CPU and memory ceilings.

## Architecture

The plugin host lives in Desktop's Tauri backend. Each installed native plugin
runs as a child process and communicates over MCP stdio. React receives only
host-validated view data and renders it with Buzz-owned components.

```text
React UI
   │ typed Tauri commands
   ▼
Desktop Tauri plugin host
   │ one bounded MCP stdio child per operation/caller
   ├──────── plugin adapter
   ├──────── plugin adapter
   └──────── plugin adapter
   │ bounded host resources and named actions
   ▼
Buzz relay and external publisher services
```

The host negotiates MCP revision `2026-07-28` and a separate versioned Buzz
contract. Discovery, tool inventory, and every call use the exact negotiated
extension metadata. The host pages `tools/list`, requires the advertised tools
capability, and compares every listed tool with the signed manifest registry.
Missing, extra, duplicate, or cross-bound tools fail installation.

Every call has an object containing a bounded immutable `context` member and
exactly one role member: `render`, `arguments`, `operation`,
`reconciliation`, `payload`, `check`, or `migration`. Each role has its own
input and output schema. The host validates results before rendering,
persistence, or dispatch.

The host uses deadlines, bounded output drains, cancellation, and process-tree
cleanup. A normal child crash, hang, malformed frame, oversized frame, or
restart loop fails the affected operation and does not stop Buzz. A child that
leaves its process group is reported; version 1 does not claim containment.

## Trust and authority

Installed native plugins are trusted code. Process separation handles ordinary
crashes and hangs but is not a security boundary: same-user code can read local
files, sockets, keychain state, and the fallback Buzz identity file, and can
use the user's network authority. Publisher trust is the version-1 control.

The host owns:

- package and manifest validation, trust roots, policy, deny rules, and the
  install/update reconciler;
- identity, signing, relay writes, authorization, user confirmation, and audit;
- community scoping, context construction, source revisions, capture IDs,
  truncation, cursors, and byte budgets;
- operation/job records, nonces, generations, receipts, idempotency keys,
  runtime and credential-binding versions; and
- the renderer, Markdown sanitization, link opening, confirmation, and
  accessibility semantics.

The plugin owns its analysis and provider calls. It receives only resources,
fields, settings, cache prefixes, selectors, and secret slots named by its
manifest and allowed by the installation grant. It never receives a signing
key, relay credential, database handle, grant list, or unrestricted Tauri
invoke bridge. It cannot call back into Buzz through the plugin contract or
register relay kinds. This does not stop same-user native code from independently
reading credentials, talking to the relay or providers, causing provider side
effects outside `execute`, escaping its process group, or suppressing a local
deny decision. The install and management UI must state these limits and treat
publisher selection as a trust decision.

Each child environment starts empty. The host adds only fixed locale and
platform-temporary-directory variables plus the calling contribution's fixed
slot variables. It never injects `BUZZ_PRIVATE_KEY`, `BUZZ_RELAY_URL`, or
`BUZZ_AUTH_TAG`. Raw stdout and every decoded string leaf in results, content,
and cache writes are scanned for registered credential values and token
prefixes. Stderr and error text are retained only after redaction. Host audit
rows contain digests, IDs, outcomes, and redacted error codes, never bodies or
credential values.

## Package and manifest

A `.buzz-plugin` is an immutable ZIP containing `manifest.json`, `schemas/`,
`LICENSES/`, and a target-specific executable under `bin/`. It has no
installer, post-install script, or package-supplied asset. The manifest
contains:

- package format version, reverse-domain plugin ID, name, description, version,
  publisher, license, homepage, source, and privacy notice;
- Buzz and contract version ranges;
- `runtime.type: "stdio"` and per-target executable path, digest, and byte
  length;
- a contribution set and exact tool registry, including role, contribution,
  input-schema digest, and output-schema digest;
- grants, settings schema, secret-slot catalog, cache version, and lifecycle
  check/migration callers; and
- marketplace-indexable attribution without requiring marketplace services.

Unknown required manifest fields, runtime types, contribution kinds, tool
roles, grant names, slot scopes, or schema dialects fail installation. Unknown
optional properties are retained and ignored.

Archive publication is atomic. TUF target metadata verifies complete compressed
length and digest before ZIP parsing. After extraction, the host verifies every
manifest-bound file, schema, and executable hash. Publisher delegation owns the
target path; target path, manifest publisher, plugin ID prefix, target triple,
grant digest, and target metadata must agree. Extraction then uses a fresh inert
directory, accepts only regular files/directories, rejects links/devices/FIFOs/
sockets and mode metadata, and makes only the selected entrypoint executable.
Paths are UTF-8 `/` paths normalized to NFC; traversal, absolute paths, drive
or UNC prefixes, NULs, alternate-data-stream suffixes, trailing dots/spaces,
device names, conflicts, and duplicates after NFC/case folding fail closed.

Admission limits are 256 files, 256 MiB compressed, 1 GiB expanded total, and
256 MiB expanded per file. Schema limits are 256 KiB per file, 2 MiB total,
10,000 nodes, depth 32, 1,000 properties per object, reference depth 8, and
a two-second compile deadline. Per-call validation has 1 MiB input/result
limits, a 200 ms evaluation deadline, regexes capped at 1,000 characters and
64 per schema, and runs in a cancellable worker.

Schema digests use RFC 8785 JSON Canonicalization Scheme over UTF-8 bytes and
SHA-256. Network references, unresolved cycles, duplicate JSON members, lone
surrogates, and numbers outside the finite I-JSON binary64 domain are rejected.

## Contributions

The public contribution categories are closed in version 1:

### Pages and entity sections

A `page` uses a host route under `/plugins/<plugin-id>/<page-id>` and a host
navigation entry. An `entitySection` appears in one of the fixed review,
repository, or project insight locations. Both declare one render tool,
resources, fields, settings, secret slots, cache access, and selectors.

Views contain text, sanitized Markdown, notices, provenance, advisory metrics,
evidence, tables, host action references, and explicit empty/loading/stale/
partial/unavailable/error states. Raw HTML and images are removed. Links are
limited to `https` and `mailto`; Buzz shows the destination before opening it.

### Actions

A core action is a host-owned named action with no plugin execution during the
action. Version 1 accepts only `project.review.approve@1`, with inputs review
ID, expected commit, and decision. Tauri rechecks open/draft status, actor !=
author, owner or requested-reviewer authority, and the exact current commit
immediately before signing. Tauri constructs the event kind, content, and tags;
plugin output never reaches the generic signer.

An external action declares an entity, argument schema, view reference fields,
and prepare, execute, and reconcile tools. Prepare returns an opaque provider
revision. Execute supplies that revision to a provider-side conditional
mutation. A mismatch returns `Conflict` with no side effect. The host records
the complete frozen input before dispatch.

### Background handlers

A handler declares semantic host topics, resources, fields, schedule limits,
settings, slots, cache access, billing, and retry behavior. It has no UI and
cannot call an action tool. The host creates one delivery ID for a topic
delivery or schedule tick. Automatic retry is allowed only when the adapter
declares delivery-ID idempotency and passes a crash-after-provider-success
fixture; otherwise the result requires reconciliation.

### Lifecycle callers

`check` validates the installed runtime and bindings. `migrate` receives a
bounded cache page and host cursor. Both are manifest-declared callers with
the same grant and selector rules as UI and background callers.

## Context, resources, and state

The host builds a bundle under one `captureId`. Every resource carries a source
revision and correlation keys. If a source advances during assembly, the host
re-reads once; a second disagreement returns `stale-capture`. There is no
cross-system transaction across relay, Git, and publisher state.

Version-1 resources are:

- `project.review@1`: review, project/repository refs, author, reviewers,
  current commit, prior decisions, and viewer authority;
- `project.review.diff@1`: bounded files and patches, binary markers, and
  truncation reasons;
- `project.repository@1`: repository and branch references;
- `project.ownership@1`: member-visible owner and maintainer facts;
- `project.reviews@1`: bounded review-summary pages with stable cursors; and
- `project.repositories@1`: bounded repository pages with revision and gap
  facts.

The existing 250-file and 2,000-line diff limits remain, with an aggregate
byte cap added before plugin dispatch. Non-truncatable identity, revisions,
correlations, authority facts, and truncation facts always fit or fail the
call. Diff bodies, cache values, table rows, and optional prose truncate in
that order with reasons.

Settings are host-owned and validated against a signed schema. Cache and
durable core records are separate. Selectors are either one exact key template
or one lexical prefix page. Substitutions come only from declared entity or
resource correlation fields. A cursor binds caller, installation, identity,
community, plugin/contract/cache versions, selector, and last key. A cache
write outside the caller's declared prefixes fails with no writes.

Secret slots have fixed host-derived environment names and bounded values.
Scope defaults to identity plus community. Rotation creates an immutable
binding version. Installation scope requires a named use case, policy
allowance, and disclosure. Settlement retains the exact binding version used
by the original operation.

## External actions and recovery

The host atomically coalesces a nonterminal match on installation, actor, community,
contribution, entity, and expected revision before assigning a nonce. It persists `Attempting` and the complete input before any external
dispatch. Every attempt ends in `Succeeded`, `Failed`, or `Unknown`.
Invalid or lost post-dispatch results become `Unknown`; they never silently
become success or disappear. A manual retry requires fresh confirmation and a
new generation. A same-key replay is allowed only under the declared provider
idempotency contract.

Durable operation and job records retain runtime reference, plugin/contract/
cache versions, settings, source revisions, argument digest, credential
binding, idempotency key, receipt, and reconciliation evidence. Uninstall and
disable retain nonterminal billable/mutating work and grant only settlement
reconciliation or one same-key replay. Dead-letter status alone is not
resolution.

When Desktop reopens, live intake starts before bounded collection reads.
The exact order is: register a durable live buffer, await subscription-ready,
read bounded current state, record event IDs, deduplicate and drain the buffer,
then continue the same subscription. Cursors remain host-owned. Work is
bounded and views disclose stale or partial state. No completeness claim is
made for events published while Desktop was closed. On startup, surviving
`Attempting` rows move to `Unknown` before new dispatch; settlement uses only
the recorded reconcile or same-key replay. An identity or community switch
discards only read-only or proven pre-dispatch work. A dispatched attempt becomes `Unknown` and settles only under its originating identity and community.

## Default safety budgets

These are version-1 defaults. Policy may lower them; rollout gates remeasure
them on each supported platform.

| Resource | Default |
|---|---:|
| MCP frame | 1 MiB |
| Cumulative output | 4 MiB or 128 frames |
| Stderr | 64 KiB |
| Deadlines | discovery 5 s; health 10 s; shutdown/section 2 s; page/prepare 10 s; execute/reconcile 30 s; background 120 s |
| Concurrency | 2 per plugin, 8 total |
| Pending jobs | 64 |
| Retries | 3 at 1/2/4 s, pure or pre-dispatch only |
| Plugin cache | 100 MiB |
| Package disk | 2 GiB, including retained runtimes and snapshots |
| Audit storage | 100 MiB / 90 days, with management reserve |
| TUF metadata | 1 MiB per file; 8 MiB per refresh; 32 roles; delegation depth 4; 32 roots per refresh, each root persisted |
| Archive path | 4,096 bytes total; 255 bytes per segment |

## Distribution and policy

Open-source Buzz has no repository URL, trust root, private key, plugin ID, or
rollout rule in its defaults. An OSS user imports a repository descriptor and
initial root, verifies the displayed fingerprint out of band, and consents to
the publisher's trusted native code before installation. The repository URL
cannot authenticate its own initial root.

A private build authenticates its catalog and policy with a verifier embedded in
the platform-signed build or an authenticated operating-system management profile.
Policy cannot add a repository URL or root. TUF verifies the root chain, delegated
policy role, target metadata, and separate emergency-deny role. Policy names catalog
IDs, versions, required state, maximum grants, and rollout cohorts. At startup and bounded refreshes, the reconciler automatically
installs or updates plugins required by valid policy. Publishing a target alone
does not install it.

Deny metadata is monotonic and evaluated before launch. Only a newer valid set can
remove an entry; the last verified set stays latched through expiry, loss, or
verification failure. Stale deny data blocks installs and updates. Deny wins over
policy and consent. Expired policy blocks installs, updates, and grant increases but
does not stop an installed non-denied plugin. Verification errors remain visible and do not block core Buzz startup.

## Compatibility and lifecycle

Manifest metadata and contract versions are separate. Contract versions use
three-component SemVer and comparator-pair ranges. Additive fields/tools are
minor changes; breaking changes increment major. Version 1 ignores unknown
optional fields and rejects unknown required enum values. Golden fixtures cover
cross-language canonicalization, discovery/list/call results, all role forms,
selectors, permissions, cancellation, budgets, recovery, and forward fields.

Updates and rollback stage a verified archive non-executable, stop new old
work, drain active calls, and retain old runtime/binding/settings for
nonterminal settlement. A forward cache change uses paged migration. Rollback
with a different cache version starts a fresh cache or uses an exact retained
snapshot; it never reverse-migrates. The host runs lifecycle checks, then
atomically switches package and cache pointers. Failure leaves the current
pointer active.

Disable stops new calls and intake. Uninstall also removes grants and
unreferenced bindings, cache, and settings. Nonterminal external work remains
visible until provider-terminal or audited manual resolution. Deny never runs
plugin code.

## Required plugins

### Code review

Migrates PR Beacon and provides an AI-generated change summary including code
diffs, a review queue, status/filter/hide parity, a review insight, core approve
action, and a billable review-change background handler. The host supplies
review, diff, and bounded queue resources. Summaries are keyed by review and
source revision; new tip commits invalidate them. Tauri owns approval
eligibility and signing. Provider replay must pass its fixture before automatic
retry is allowed.

### System stewardship

Provides review/repository insights, a project page, and a non-billable
schedule handler. It produces advisory complexity and change-risk signals with
evidence, owners, source/rules revisions, age, and partial status. Consent
selects bounded project IDs. The same validated facts feed the human view and
an optional separately capped, explicitly untrusted agent-context block.
Stewardship cannot affect approval, merge eligibility, or workflow gates.

### Loop management

Provides loops, spend views, refresh, lifecycle check/migration, and an external
archive action. Builderbot remains authoritative. Prepare returns a revision;
execute performs an atomic conditional archive. Timeout becomes `Unknown`.
Spend does not enter logs. Missing conditional mutation support blocks this
plugin's acceptance gate.

## Future probes and marketplace

CI, issue tracking, observability, incident response, deployment, and knowledge
search are future probes. Each requires a later versioned schema for its event
rate, OAuth, artifact/result size, action duration, or permission model.

Version-1 package and manifest metadata may be indexed later, but marketplace
search, hosting, reviews, ratings, payments, publisher admission, and execution
of untrusted native code are deferred. A public marketplace requires a sandbox
or provider-verified one-time capability before it can admit third-party native
plugins.

## Rollout gates and open blockers

1. Contract/trust prerequisites: exact MCP capability objects, schema
   canonicalization, fixtures, signer roles, custody, and supported targets.
2. Local runtime: atomic publication, supervisor, per-caller environment,
   selectors, durable records, startup recovery, audit, scanning, and UI.
3. UI/actions/background: safe views, six resources, capture correlation,
   conditional actions, receipts, reopen intake, and recovery fixtures.
4. Signed distribution: archive and TUF rejection fixtures, root rotation,
   policy/deny reconciliation, staged activation, rollback, disable, uninstall,
   and disk admission.
5. Code review, stewardship, and loop management: each mapping's provider
   fixtures and behavior gates, followed by a cross-language reference plugin
   and contract `1.0.0` freeze.

Actual blockers:

- Builderbot and review-provider auth, receipts, reconciliation reads, rate
  limits, error models, approval target, conditional writes, and idempotency
  fixtures;
- MCP client support for the selected revision and per-request metadata;
- signer identities, thresholds, custody, bootstrap channels, and root-rotation
  drills; and
- the supported OS/CPU signing matrix and per-platform signing rules.

The dedicated review-approval event-kind question is deferred and nonblocking.
Version 1 keeps the existing core event representation.
