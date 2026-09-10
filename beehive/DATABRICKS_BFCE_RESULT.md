# Databricks v2 local provider — bfce

Tested executable candidate: `2ea103d74` (full SHA in final-pins.txt).
Source contracts read at pinned `051c3a270be9c73da9ab06700bcab7d5552fceaa`
in buzz-reference, via git show (not its working tree).

## Behavior and source contracts

Normal/diagnostic wizard option **8**, provider **databricks_v2**, and existing
**local-setup → add-buzz-provider** now accept explicit **token** or
**external-oauth**. Token requires an absolute owner-only local secret file; the
existing provider security owner checks canonical regular file, service UID,
permissions, bounded nonempty token. Credentials are never public fields.
External OAuth forbids a token file/fallback and does not read a cache or claim
credential presence. Both accept operator-approved exact models and HTTPS
workspace origins (no credentials/path/query/fragment/custom port). Unsupported
combinations fail; no catalog/model fallback. Historical serialized
`buzz-agent-api-key` mode is retained, not a new universal credential registry.
Legacy normal option 2 remains compatible; explicit auth choice is option 8.

Config source `config.rs:631–691,922–960`: DATABRICKS_HOST is required;
DATABRICKS_TOKEN is optional, empty selects native OAuth; BUZZ_AGENT_PROVIDER is
`databricks_v2`, BUZZ_AGENT_MODEL overrides DATABRICKS_MODEL. Launch environment
is closed; no Goose/Claude/Codex auth stores or ambient provider keys inherited.
Token binding supplies DATABRICKS_TOKEN only from the protected file. External
OAuth supplies no DATABRICKS_TOKEN. Existing service HOME and config directory
remain local and fixed in the immutable binding.

`lib.rs:151–200`: native `buzz-agent auth databricks_v2` alias is supported,
reads DATABRICKS_HOST, requires a browser on that machine. Setup and auth-info
print shell-quoted manual env context under the actual service OS user:
HOME, BUZZ_AGENT_CONFIG_DIR, DATABRICKS_HOST and installed executable. They do
not run auth or inspect credentials. `auth.rs:1287–1392`: later native runtime
bearer acquisition/refresh is headless; cache is workspace-discovery/client/scopes
specific under BUZZ_AGENT_CONFIG_DIR/buzz-agent/oauth/databricks, default
~/.config/buzz-agent/oauth/databricks without override. Guidance explicitly says
not to copy personal caches. Locally configured != credential present != verified
login/refresh/model readiness. Actual ACP errors remain fail-closed; no claim
of rich native auth error-code classification from these fixtures.

Same host/identity/tool/relay/trust owners, immutable binding replacement/retirement,
lock/snapshot fencing, public model/workspace/profile selection and explicit
Start/Restart reused. No automatic binding selection, Restart, sibling mutation,
key recreation, or standby permission. No real vendor/native login/probe performed.

## Executed workflows

- Databricks token: normal wizard → real host CLI → actual TUI Start/Restart →
  real installed buzz-acp/Buzz CLI with source-shaped TS harness.
- Databricks external-OAuth **configuration fixture**: same journey. Fixture
  permits conversation without native cache access and separately denies auth and
  simulates missing refresh. This is NOT authenticated Databricks evidence.
- Provider switch/conversion: initial diagnostic **Anthropic** → immutable local
  add **OpenRouter B** → public binding selection → Stop → immutable normal
  conversion B-normal/replacement → public selection → Start/Restart. Same key.

Each new journey verifies four actual signed same-agent replies, two channels and
later owner turn, fresh-session Restart/exact model/native profile, history and Y
unchanged. Wrong model/native capability/auth denial preserves actual; token wrong
or missing key preserves actual; Stop remains neutral. Public inventory excludes
private key, file, endpoint and HOME. OAuth configured/missing-refresh/denied are
fixture states, not claims about native login outcomes. New journeys focused 3/3
(65.616s), provider units 5/5, strict PASS. All also pass in the final full run.

**Source-only variants:** Databricks local add/conversion; Anthropic and OpenAI-
compatible conversion; custom conversion. OpenRouter's add/conversion is executed,
not inferred from initial setup. Databricks real vendor OAuth/model access,
`databricks-claude-haiku-4-5` live acceptance, mesh/compute and production admission
remain separate and unauthorized here. The installed binaries are Buzz's runtime
and CLI, NOT installed Databricks/Buzz Agent inference (TS fixture instead).

## One final FULL DEFAULT gate: OPEN

Node 24.15.0 / pnpm 11.4.0, existing standalone lock/dependencies unchanged.
Strict PASS; ONE default concurrent installed-enabled package run on `2ea103d74`:
**116/117**, zero skip/cancel, natural **289.084s**. No rerun, serial masking,
forced exit, deadline/assertion change or speculative fix. Failure:
`admission-cancel.test.ts:136`, conflicting-id aggregate **2646ms** vs unchanged
<2500ms. Correct accepted Start/conflict rejection/accepted Stop receipts precede
the failed aggregate assertion. Historical 2572ms and 2612ms remain independently
unresolved, not explained away by current provider success.

Fixture-activated storage diagnostics preserve original fsync trace, adding
post-operation JSON, UTF8, write, open/close, rename, mkdir/permission-mode spans,
monotonic and process CPU, plus row-emission timing. No payloads. Process CPU is
not kernel service time; async await spans include scheduling/threadpool effects.
No synchronous trace-file output inside measured regions; bounded capture flushes
at test completion/failure. Existing permission modes/atomic durability unchanged.

Failure capture: `89238-conflicting-id.json`, **598 rows, zero dropped**. Unlike
prior Start-journal-dominant recurrence, this capture includes **766.413ms async
relay rename await** before the Start/conflicting-Stop batch becomes durable.
The fixture event loop continues ticking during that wait; this is NOT proof of
766ms disk/kernel execution. Start handler begins 938.466ms after aggregate timer;
conflicting Stop handler 1458.180ms; valid Stop handler 1731.714ms. Owned Stop is
525.619ms. Timer ends 2646.336ms (wall elapsed2646). Largest new synchronous detail
span is rename71.973ms with process CPU user20us/system1388us; another rename
69.024ms/user23us/system906us. These separate operation-associated observations,
not underlying filesystem/scheduling causes. Total measured row emission0.600ms
excludes caller, clock/CPU measurement and unmeasured instrumentation overhead.
No causal claim about earlier captures; no governing semantic defect demonstrated.

Evidence directory: workspace
`WORK_LOGS/BEEHIVE_DESIGN_183FFAF0/DATABRICKS_BFCE/`:
`focused-installed.log`, `strict-final.log`, `full-default.log`, `final-pins.txt`,
`trace-analysis.txt`, `traces/89238-{wrong-authority,conflicting-id}.json`.
Installed read-only hashes:
- buzz-acp `10612d0025d1420bdd9e9afcc2e7129377da2414e75049a8b640e81d857441ea`
- buzz `147cc2ccf276ddedc5b84d13f5399a95282066303c9dca566bb2af5a37b856c5`

Self-review covered private/public env boundary, explicit OAuth no-file branch,
shared wizard, existing immutable ownership, fixture authenticity and storage
measurement scope. No independent approval of this delta claimed. Prior scoped
reviews remain scoped. No repo CI, production provider access, native/Rust/client
changes, release or merge. Full gate remains an explicit unresolved handoff item.
