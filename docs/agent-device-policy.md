# Agent hosting across desktops

Settings → Agents → Agent hosting → Unique agent names lets this device host
new agents while protecting existing identities hosted elsewhere. With
`unique_names: true` and `client_only: false`, the preferred remote names,
public keys and optional `persona_id` bindings cannot be created, started,
renamed, deleted or managed locally. An unrelated name can be created and run.
The ordinary Client-only switch still disables all local hosting.

Hosting reservations apply across this installation's local agent catalog,
including after switching accounts or communities. A protected name remains
reserved locally, and a copied protected public key or definition ID cannot be
started by changing the active community. The `relay_url` and `owner_pubkey`
fields below scope discovery preferences; they do not create separate local
hosting namespaces. Hosting unrelated same-name agents in different communities
would require a separate change to the local catalog's collision rules.

Existing-identity protection requires explicit `preferred_agents` bindings in
the device policy file described below. Settings does not infer which computer
should host a copied record. An empty binding list still checks new names for
collisions, but **does not stop existing local agents or copied autostart
records**; Settings discloses this state. Use client-only mode for a secondary
device until its protected identities have been configured. To configure
bindings, quit Buzz, preserve the current policy file, and set the existing
host's exact public keys and definition IDs in `preferred_agents`. Do not copy
private keys or delete shared definitions to configure this policy.

New identity creation and renames are serialized on this device. Checks reject
an existing local instance, a different same-name definition, or an exact
same-name profile with verified ownership on the active relay. An unavailable
or incomplete relay lookup refuses the operation; offline presence does not
free a name. This is not an atomic cross-device reservation: simultaneous
creation by another unconfigured client still requires relay-side coordination.
Edits that keep the same name still enforce the protected-identity guards but
do not require an online directory lookup; local credential, prompt and model
configuration can therefore be saved while the relay is unavailable.

Unique-name mode keeps runnable definitions and team templates local, including
their old pending backlog. Only lifecycle records for explicitly authored local
identities are published: kind:30177, its deletion and its archive request. A
durable key registry in the scoped retention database permits those operations
to retry after deletion/restart without releasing unrelated queued events.
Public agent profiles and ownership policies remain visible to the other client
for channel invitations and mentions. Individual agent imports are supported;
team imports and catalog publication require unrestricted hosting.

The discovery-clear control is unavailable while unique-name protection is
enabled. The native settings command also refuses removing protected bindings
while retaining unique-name mode. Disabling both restrictions resumes normal
synchronization after restart, including retained edits/deletions.

A desktop can use the same account as an agent host while acting as a chat
client. Settings → Agents → Agent hosting → Client-only mode is local to that
installation. It takes effect after restarting Buzz. The hosting desktop keeps
its default setting.

In client-only mode, Desktop refuses agent key generation, instance imports,
starts, deployment and agent/team definition management. It exposes no local
runnable inventory and projects definitions as inactive without saving that
projection. Existing relay identities remain available for messages and invites.
An offline host does not permit creating a replacement agent.

Automatic outgoing agent/persona/team reconciliation and queue flushing are
paused. Existing queues and records are retained. Inbound signed changes still
update local projections, without catalog echoes or runtime refreshes. This
prevents a stale local deletion from being published when an observer reconnects.
Review retained local state before returning a previously repaired observer to
hosting mode; reenabling restores normal synchronization after restart.

The preference lives in `agents/agent-device-policy.json` under this desktop's
app data directory, independently of `managed-agents.json` and relay sync:

```json
{
  "client_only": true,
  "unique_names": false,
  "preferred_agents": []
}
```

Optional `preferred_agents` entries contain `relay_url` (canonical HTTP URL),
`owner_pubkey`, `name`, `pubkey`, and optionally `persona_id`. They select an existing identity within
that owner/community for name-based discovery. They neither mint keys nor grant
access. Older same-name identities are omitted from new selections; their
history, profiles, explicit public-key links, and send-time authorization remain
exact. When unique-name protection is disabled, Settings can clear discovery
preferences without deleting agents.

The active policy, including read errors, is fixed for the process lifetime.
The native boundary refuses execution on a malformed/unreadable policy; Settings
can reset it to client-only mode and apply that recovery after restart. A missing
file preserves existing hosting behavior. The file is bounded to 64 KiB and
written atomically with restricted permissions.

This is a device policy, not a relay-wide lease: another unconfigured desktop
can still create agents. Installations that must only observe must use this
client feature. Preserve the device policy across application upgrades; a client
version predating this feature does not enforce it.

Regression coverage: `managed_agents/device_policy` native tests; native
deployment payload refusal; `AgentHostingSettingsCard.test.mjs`; and
`client-only-agents.spec.ts` with the upstream remote mention and invite suites.
