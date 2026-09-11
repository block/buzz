# Public agent metadata (separate from private instructions)

Public kind0 contains **only** `display_name`, `picture`, `about`. Do not put
behavior instructions, private inventory, provider details or credentials in
these fields. Empty fields clear the previous public value; `about` is at most
280 Unicode characters. Picture is an HTTPS URL, not an upload operation.

## Edit without a host, network or key

Run `beehive drafts <owner-state-directory>` (the directory containing
the owner TUI identity reference). Use `metadata-new`,
`metadata-edit <draft-id>`, `metadata-discard <draft-id>`, `metadata-drafts`, `quit`.
Discard only removes a local draft, never a submitted operation or public event. Enter the exact retained
relay URL and agent **hex public key**, then the three public fields. The final
valid about line + Enter atomically saves the whole form. Partial forms are not
saved; closing mid-form retains the previous saved version. Blank input clears a
field, not “keep previous.” Resume prints the saved values before prompting.

Drafts live in `metadata-drafts/drafts.json` (owner-only 0700/0600) under owner
control state, **not on the current host**. The separate `profile-drafts` store
and immutable private instruction library are untouched. Concurrent stale edits
fail instead of overwriting another save. There are at most 100 drafts. The
short directory edit lock is the same mechanism as instruction drafts: after a
crash, confirm no live editor before locally removing a leftover `edit.lock`.

## Deliberate publication while the model is stopped

In the private owner TUI, the same draft commands work. Select the exact agent
on its currently assigned eligible host, then `metadata-publish <draft-id>`.
Review the complete PUBLIC payload and confirm `yes`. No Start, Restart, ACP
executable, model or provider readiness is required. Stale/unreachable host
inventory prevents submission and retains the draft. Host disappearance after
submission leaves an owner journal operation pending/unknown.

The private management command is owner-signed and encrypted. The host consumer
checks the signer (transport), host/agent/relay binding, retained assignment,
operation identity, host CAS revision and exact metadata hash before reading the
selected agent credential. The host key is infrastructure, never the public
profile signer. Only the existing agent key signs both real kind0 and the NIP98
request authentication for the relay's supported `POST /events` and `/query`.
The retained conversation relay must equal the management relay, and a valid
**existing** agent-owner NIP-OA association must be present in that local
conversation binding. Its signature, owner, agent and conditions are checked;
no attestation is created, copied from another agent, or repaired. Conditions
restricted to a single kind cannot authorize both kind0 and kind27235 and are
refused. Missing association/key, removed key or denied credential access is an
explicit recoverable error; no key regeneration or owner-key access occurs.

## Draft, pending, published, failed/unknown

`metadata-drafts` lists saved desired values and matching submitted-operation
statuses; `operations` shows all attempts. A saved draft is never evidence of
publication. The existing owner `management-intents` journal durably retains the
exact operation before sending. The host retains the exact agent-signed candidate
in its per-agent journal's `publicMetadata` field before submitting it. This is
an attempt record, **not a new editable agent definition**.

Only a positive matching event ACK **and** a signature-verified latest kind0
readback with that exact event ID produce `published; signed latest kind0 <id>`.
A receipt records a historical observation; later external edits may supersede
it. The published representation lives on the community relay as the latest
kind0 by the agent pubkey. Public readback never replaces the owner's draft.

`reconcile` reopens management transport and retrieves retained receipts. Duplicate
operations never repeat the public side effect, including after a host crash.
If interruption loses the terminal observation, status remains unknown; this
bounded slice does not automatically republish or turn queue persistence into
success. Check the desired draft, wait for fresh host state, then deliberately
submit a new publish attempt if needed. A same-second retained signed attempt or same-second/future-dated latest kind0
is refused rather than inventing a timestamp or gambling on Nostr event-ID tie
ordering; wait until its timestamp is past before retrying. Stale host revisions
and old operation IDs cannot overwrite a newer accepted edit. ACK refusal,
readback mismatch and network failures remain visible, not silently retried.

Publication changes no selected instructions/configuration, actual model run or
sibling agent profile. It consumes the host operation revision on success, like
other serialized owner actions; use refreshed inventory for the next action.
No generic Move, key recovery, membership, invite or provider subsystem is added.

## Source and validation scope

Native contract reference is immutable Buzz `051c3a270be9c73da9ab06700bcab7d5552fceaa`:
Desktop `relay.rs` `build_profile_event`, `sync_managed_agent_profile`,
`query_relay_at_with_keys`, `query_agent_profile`; `commands/agents_profile.rs`;
SDK `nip_oa.rs`; relay `api/bridge.rs`. At this pin Desktop's managed-agent
builder/query actually use name/picture, not about. Beehive adds the requested
explicit about field through the supported kind0 JSON contract, not through
behavior text.

`test/public-metadata.test.ts` drives the real offline CLI, private management
consumer and signature-enforcing loopback HTTP consumer with synthetic keys and
an explicitly isolated credential adapter. It checks stopped publication with
nonexistent ACP/provider executables, negative ACK/readback, wrong owner/relay,
malformed public payload, stale revision, missing key and timestamp ordering.
This is source-shaped relay evidence, not a production relay, OS credential,
installed-native or live-person profile publication claim.
