# First private demo (owner-run, not yet a live demonstration)

The tested path is **public `host` CLI → private owner `tui` → Start → agent-signed
reply → Stop**. Automated evidence uses a loopback membership-enforced relay model,
a synthetic credential helper, the installed Buzz ACP/CLI and a deterministic
provider harness. It proves neither macOS Keychain readiness nor deployment/provider
compatibility. Do not use frozen or existing installations for this demo.

## Prerequisites

- Node 24.15.0 (any Node.js 22.18+ works; the `beehive` launcher checks the
  version and says what to do) and this package's frozen standalone
  dependencies. Install the command once, reversibly, into a user bin directory
  already on your PATH, matching nothing else:
  `ln -s /absolute/path/to/this/checkout/beehive/bin/beehive.cjs ~/.local/bin/beehive`
  (adjust the target to a user-owned bin directory that is on *your* PATH;
  undo with `rm`). Then run `beehive <command>` from any directory. Advanced
  fallback inside the package: `/absolute/path/to/node src/cli.ts <command>`;
  no root bootstrap is needed either way.
- `beehive setup` with no directory uses the default host state folder
  `~/.beehive/host`, resolved from your home (never the current directory) and
  displayed once before the wizard. An occupied default is never silently
  reinitialized; pass an explicit directory for additional hosts. Owner
  approval/catalog files below stay explicit and never land in host state.
- A deliberately chosen **new** 0700 host directory, a dedicated host service OS
  user with an interactive unlocked credential store, and a separate owner context.
  `@napi-rs/keyring` 2.0.0 may show OS approval/unlock prompts; it cannot suppress
  them. Only `beehive` service entries `host:<public-key>` and `agent:<public-key>`
  are used. Never approve access to someone else's account/cache.
- Existing owner signer and an explicitly supplied demo agent key/authorization.
  Agent enrollment/channel membership on the conversation relay is a separate
  prerequisite: host registration is not an agent-owner grant.
- Relay operator has already enrolled the host and owner public keys as ordinary
  direct members. This CLI currently supports **already-member verification**;
  it does not mint invites or expose invite claims. The relay must advertise its
  `self` key and NIP-43, with membership-gated signed `POST /query`, NIP-42 and
  NIP-59 giftwraps. Open-relay AUTH/ACK and a saved roster are not admission.
- One installed supported provider configuration. The example below chooses Buzz
  Agent / Databricks v2, dedicated service HOME/config, plus installed buzz-acp and
  sibling Buzz CLI. Real provider/native protocol compatibility remains unverified.
  Do not substitute a different provider until its configuration is intentionally
  supplied. No login, inference, invite or production enrollment was run by workers.

## Manual setup → Start → reply → Stop

1. **Host, owner present:** `beehive setup`
   (default host folder `~/.beehive/host`; `beehive setup <explicit-directory>`
   for an additional host). Choose `create`, supply label, owner's **public** hex key and `wss://relay-host`;
   choose a new private pairing file. Expect OS key creation/readback approval.
   If interrupted, use `export` on the same directory; never regenerate identity.
2. Transfer the pairing file privately to the owner. **Owner context:** run
   `beehive setup /new/owner-exchange` (an explicit directory keeps owner
   approval files outside host state), choose `approve`, select the pairing,
   compare the full fingerprint with the host, confirm `yes`, enter a future Unix
   expiration and a new private approval file. Enter the existing owner key only
   at the hidden prompt. Transfer the approval file privately back to the host.
3. **Host:** rerun `beehive setup` on the same default host folder, choose `import`, select
   that approval. This registers infrastructure only. Arrange the **already-member**
   prerequisite above out of band; there is no administrative operation here.
4. Prepare a private local `binding.json` (absolute paths, no identity keys):

   ```json
   {
     "mode": "buzz-agent-databricks-v2",
     "runner": "/absolute/supported/buzz-agent",
     "args": [],
     "workspace": "/absolute/demo-workspace",
     "serviceHome": "/absolute/dedicated-service-home",
     "configDirectory": "/absolute/dedicated-service-config",
     "databricksHost": "https://your-workspace.example",
     "conversation": {
       "executable": "/Applications/Buzz.app/Contents/MacOS/buzz-acp",
       "relay": "wss://your-conversation-relay",
       "replyTool": { "executable": "/Applications/Buzz.app/Contents/MacOS/buzz" }
     }
   }
   ```

   Supply the owner's deliberately approved **initial demo placement** public
   genesis file: `{ "v":1, "id":"<fresh UUID v4>", "owner":"<owner hex>",
   "agent":"<demo agent hex>", "initialHost":"<paired host hex>" }`.
   This is a locally pinned root, not a signed bearer grant. It must match the
   intended agent and host. **Never replace an existing agent's retained genesis
   or invent a new root to bypass its assignment**; use a separately authorized
   initial demo identity/placement, not migration or Move.

   Run `beehive provision-agent ~/.beehive/host /private/binding.json /private/genesis.json`.
   Enter the matching agent key at the hidden prompt. Expect OS create/readback
   prompts. The owner key is never placed on the host. If interrupted, retain all
   artifacts and use `reconcile-provision` with these exact same inputs, not a new key.
5. Run `beehive auth-info ~/.beehive/host` for the exact service-user
   native login context. Deliberately configure/login the supported provider as
   that user only. No borrowed Desktop cache. Presence of files is not login proof.
6. **Owner:** `beehive catalog /new/private-catalog.json /private/approval.json`.
7. **Host, owner present:**
   `beehive host wss://relay-host --owner-present`
   (default host folder; the explicit `beehive host <host-directory> <relay>
   --owner-present` form remains available). The flag consents to live OS reads, not relay admin or provider login. Each key
   read uses an owned helper with a 10s deadline; SIGINT/SIGTERM cancels and awaits
   its actual close. An OS-owned dialog may remain after helper termination;
   inspect/dismiss it deliberately. Denied/timeout is not a missing key. Expect a
   fresh host-signed HTTP membership check, then `Host online`.
8. **Owner:** `beehive tui /private/private-catalog.json wss://relay-host`.
   Enter owner key at the hidden prompt. A separate owner-signed membership check
   precedes private transport. Use `hosts`, `select <full host hex>`, `show`, then
   `start`. Wait for `completed | accepted` and running actual state; publication
   alone does not prove Start. From the owner's usual Buzz client, mention the demo
   agent in its authorized conversation channel and observe its signed reply.
9. In the TUI, `stop`; wait for accepted receipt and `show` stopped/no actual run.
   `quit` closes only the owner UI. Ctrl-C the foreground host for owned teardown.
   Missing provider credentials do not prevent Stop. Preserve journals if teardown
   reports uncertainty; do not clear locks or kill unrelated PIDs.

## Honest first-demo limits

Membership proof is fresh and single-use for one connection (10s admission window),
not indefinitely cached. After transport loss, no private automatic reconnect:
close/reopen the command to verify membership again. `reconcile` explains that
requirement instead of silently reusing proof. A live socket's revocation visibility
is governed by relay policy; this is not continuous membership monitoring.

Roster timestamps are not compared with local claim time. A fresh membership-gated
row check can pass with an old or lagging roster. Failed admission does not fall back
to public management, owner OA, another signer, or plaintext storage. Registrations,
catalogs, management inventory and lifecycle commands remain private. Move, profile
distribution and generalized recovery are outside this first demo.
