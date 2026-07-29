# External Relay Agents in Desktop Agents

## Context

Buzz Desktop v0.5.0 currently renders the Agents page from `list_managed_agents`, which is the current Mac's local control-plane store. The five PoC agents run independently through `buzz-acp`, have separate identities and work directories, and are represented in Relay agent profiles and channel membership rather than the Desktop managed-agent store.

The result is correct runtime isolation but incomplete visibility: the agents work in channels and mentions yet do not appear in the Agents page.

## Goal

Show external Relay agents that the current Desktop identity can interact with in a distinct, read-only section of the Agents page.

## Non-goals

- Do not import external identities into `managed-agents.json`.
- Do not expose private keys, Runner paths, environment variables, or external logs.
- Do not add start, stop, restart, delete, edit, auto-start, or model controls.
- Do not change Relay membership, agent allowlists, channel membership, or the five PoC Runner processes.
- Do not change the application version from v0.5.0.

## Approaches considered

### A. Separate read-only Relay section — selected

Use the existing Relay agent query, filter it with the same current-user interaction rules as message mentions, exclude locally managed pubkeys, and render the remainder below the local agent library.

This preserves the control boundary and makes the five PoC agents discoverable.

### B. Import external agents as managed agents — rejected

Writing external identities into the Desktop store would imply local key custody and process ownership. Desktop could then duplicate or stop externally managed Runner processes.

### C. Treat external agents as remotely manageable — deferred

A remote control plane would need explicit capability negotiation, authenticated Runner control, audit events, and failure semantics. Relay presence alone is not sufficient authority.

## Architecture

`useManagedAgentActions` already loads all required inputs:

- `managedAgentsQuery`: local instances that Desktop may control.
- `relayAgentsQuery`: Relay profile/directory entries.
- `channelsQuery`: active channels visible to the current identity.

The hook will also read the current Desktop identity and call a pure selector:

```ts
selectVisibleExternalRelayAgents({
  currentPubkey,
  managedAgentPubkeys,
  relayAgents,
  sharedChannelIds,
})
```

The selector will:

1. Normalize pubkeys.
2. Reuse `relayAgentIsSharedWithUser` so the Agents page and mention menu agree on eligibility.
3. Exclude pubkeys present in the local managed-agent set.
4. Deduplicate Relay entries by normalized pubkey.
5. Sort `online`, then `away`, then `offline`, and sort equal statuses by display name.

The Agents page will pass the selected list to a focused `ExternalRelayAgentsSection` component.

## UI behavior

- Heading: `Relay agents`
- Description: `Agents running outside this Desktop. Status and channel sharing are read-only here.`
- Each card displays the Relay profile name, avatar when available, `External · N channels`, and its Relay status.
- Clicking a card opens the existing profile panel.
- There are no card action menus or runtime buttons.
- The section is hidden when the query has completed with no eligible external agents.
- While loading, the section shows a small card skeleton.
- Query failures render an inline error inside the section without breaking local agent management.

## Security boundary

The UI is informational. It consumes public Relay agent metadata already used by channels and mentions. No local managed-agent record is created, and no external control command is introduced.

## Tests

The pure selector must prove:

- eligible external agents are returned;
- local managed agents are excluded case-insensitively;
- unshared or non-invocable Relay entries are excluded;
- duplicate Relay profiles collapse by normalized pubkey;
- status and name sorting is deterministic.

TypeScript checking and the full Desktop test suite must pass. The release build must remain v0.5.0, and the installed app must be verified with the five live PoC agents visible under `Relay agents`.

## Acceptance criteria

1. Product, Tech, Coding, CR, and QA appear in the Desktop Agents page.
2. Their status and channel count are visible.
3. They do not expose local start, stop, edit, delete, model, key, or log controls.
4. Existing local agents remain unchanged and are not duplicated.
5. Channel membership, mentions, Relay history, and Runner processes remain unchanged.
