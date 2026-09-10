# Buzz Agent API-provider slice — 16b4

Executable candidate: `36e2eaca0` (preceding implementation `575254ac9`).
Pinned runtime/config source: `051c3a270be9c73da9ab06700bcab7d5552fceaa`.

## Delivered scope

Three distinct Buzz Agent API-key contracts: Anthropic, OpenAI-compatible,
OpenRouter. Normal setup option 8 and local immutable add-buzz-provider share
source-grounded input. Credentials, endpoint, wire selection and service paths
remain local. Public model/workspace/binding selection, profile/configuration CAS,
Start/Restart use existing owners. No automatic selection/restart/login/install.
Existing Claude/Codex API-only restrictions and ten diagnostic presets unchanged.

Config consumers: `crates/buzz-agent/src/config.rs:631–691,922–960` at pinned
source. Anthropic key/base URL; OpenAI **COMPAT** key/base URL plus auto/chat/responses;
OpenRouter key/base URL, Chat only. Universal `BUZZ_AGENT_MODEL` overrides provider
model defaults. No invented generic env map or native profile transport.
Databricks `lib.rs:151–200`, `auth.rs:1287–1325,1356–1392` read: explicit native
service-user login with DATABRICKS_HOST, headless runtime refresh, cache identity
by discovery URL/client/scopes. Default service HOME `.config/buzz-agent/oauth`,
override root `BUZZ_AGENT_CONFIG_DIR/buzz-agent/oauth`. No auth stores read.
New Databricks token setup/richer OAuth diagnostic guidance NOT delivered.

## Validation (not vendor inference)

Node **24.15.0**, pnpm **11.4.0**, standalone existing locked dependencies;
no install/config changes. Strict PASS. Provider input/launch negatives **3/3**.
Focused initial normal installed journeys **3/3**, 58.527s. Real wizard → host CLI
→ actual TUI → real installed buzz-acp/Buzz CLI with source-shaped TypeScript
provider harness: four signed same-agent replies each, two channels/later turn,
fresh-session Restart/exact profile/model, sibling/history/manifest unchanged.
Final suite additionally passes wrong private credential, missing credential,
model/native rejection and neutral Stop controls; public inventory excludes local
key/path/HOME/endpoint. Unit checks cover endpoint/wire/mixed-contract/unsafe-file
rejection and all three OpenAI wire mappings. No real provider network inference.
Local add/diagnostic conversion is implemented but not an executed new installed
journey; custom conversion also remains source-only.

One FULL DEFAULT concurrent installed-enabled suite (no concurrency override):
**111/112**, zero skips/cancellations, natural **223.753s**. All ten installed
normal journeys PASS. No rerun, timeout change, force exit or weakened assertion.
Gate remains **OPEN**. Failure is admission control **conflicting-id**, aggregate
2612ms versus unchanged <2500ms. Earlier wrong-authority control passed at 1712ms;
later controls in that test were not reached. This is not a provider test failure
and not evidence that the original 2572ms wrong-authority failure is classified.

## Durable owner evidence at this failure

Original instrumentation-complete.patch SHA256
`37de1d9fd5110017a8be3caf739a5b3a4cf56ee5e512b77ea1d7619580f84129`
reused/adapted narrowly. Trace inactive normally. Test activates with
`BEEHIVE_LATENCY_TRACE=<fresh private evidence directory>`; bounded 50k rows,
no payloads/credentials/results/URLs, exclusive 0600 file+directory fsync on
completion/failure, t.after fallback if cleanup fails. No operational timing or
authority policy change. Exact source and binary pins retained separately.

Conflicting-id trace has **472 stamps, zero dropped**. Relative monotonic ms:

| Owner event | ms |
|---|---:|
| Start reaches slot handler | 153.806 |
| Start receipt seal | 1344.754 |
| Conflicting Stop handler / conflict receipt seal | 1348.277 / 1348.289 |
| Conflict receipt first decoded | 1625.343 |
| Valid Stop submitted / handler | 1642.016 / 1759.741 |
| Owned Stop begin / end | 1915.733 / 2447.602 |
| Valid Stop receipt first decoded | 2593.462 |
| Aggregate timer end | 2611.454 (wall elapsed 2612) |

A synchronous **writePrivate(journal.json) span of 582.982ms**, 761.769–1344.750,
blocks this event loop during legitimate Start completion. File fsync itself is
19.850ms and directory fsync 18.321ms; **do not call the whole span fsync time**.
Before file-sync begin: 392.743ms; between file-sync end and dir-sync begin:
152.055ms. Those subregions still combine filesystem calls and scheduling without
finer stamps. Maximum timer lateness is 582.785ms. Concurrent async relay
file.write await is 615.456ms and overlaps that synchronous blocking region; it
is NOT established as 615ms disk execution. Owned Stop is 531.869ms (existing
500ms containment policy), not rejection latency. Conflict handler-to-seal is
0.012ms after Start; unknown-agent bypass in the original remains distinct.

Thus this recurrence has an event-associated owner-level decomposition, including
a dominant synchronous journal region and legitimate Stop contribution. Underlying
OS/I/O scheduling cause remains unresolved; no speculative fix. Original admission
failure remains unresolved independently. Next engineering action is review this
captured failure at journal/write/scheduling owners, not another blind full suite.

## Evidence and boundaries

Workspace `WORK_LOGS/BEEHIVE_DESIGN_183FFAF0/PROVIDER_16B4/`:
`full-default.log`, `strict-final.log`, `final-pins.txt`, `trace-analysis.txt`,
`traces/83342-{wrong-authority,conflicting-id}.json`, all focused iteration logs,
`instrumentation-adapted.patch`, `post-run-processes.txt`.
First journey failed because fixed fixture session ID violated fresh identity;
next two failed on later-session restoration (set_model, then prompt owner).
Fixture corrected without runtime changes or assertion relaxation. First strict
inference failure and failed patch application/strict iterations retained/documented.

Installed binaries, read-only:
- buzz-acp SHA256 `10612d0025d1420bdd9e9afcc2e7129377da2414e75049a8b640e81d857441ea`
- buzz SHA256 `147cc2ccf276ddedc5b84d13f5399a95282066303c9dca566bb2af5a37b856c5`

No production/current-kind40002, OAuth login/cache access, real vendor inference,
Rust/native/client changes, full repository CI, mesh/compute/live admission claim.
Custom confirmation review COMPLETE SOURCE-ONLY/no new blocker; R1 source-closed
plus actual wizard regressions. Previous scoped reviews remain closed; historical
broker/tool report complete/unclassified. No independent review of this new delta.
