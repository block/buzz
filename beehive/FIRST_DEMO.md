# First private demo: configure → host availability

On the host computer run **`beehive setup`**. Fresh setup asks only for the
existing owner's **public npub** (valid hex also works) and the management relay
URL, then deliberate OS credential consent. Host name and private state paths are
automatic. Never enter the owner's private key on a host.

**Resuming the old pairing-file prompt or an unapproved f215 installation?**
Cancel the old wizard. After the verified update, run **`beehive setup`** again,
or `beehive setup <same-explicit-directory>` if you previously chose one.
It reuses the original host key reference, owner and relay, even if an agent
manifest already exists. No create/export/import menu, approval exchange, identity
reset, credential repair or migration is required or performed. Approved
installations and their history remain usable.

## Short host route

1. `beehive setup` uses `~/.beehive/host`, never your current directory. Confirm
   host-key access only while deliberately present as the host OS user.
2. Setup is **configuration-only, NOT serving**. Resume retains the same identity,
   owner and relay without network calls or credential reads. No invite is needed
   by setup, and setup claims no verified access or membership.
3. Deliberately start the foreground host: **`beehive host --owner-present`**.
   For a nondefault folder: `beehive host <same-directory> --owner-present`.
   Startup waits for authenticated private transport and accepted availability
   publication; only `Host online` means serving. Zero agents is supported; setup starts none.
   Ctrl-C stops this foreground host; no daemon/service is installed.
4. On the **trusted owner computer** run
   **`beehive tui discover <management-relay> ~/.beehive/owner`** and provide the
   existing owner signer at its hidden prompt. `hosts` shows private available infrastructure; `agents`
   shows actual agent inventory separately. No approval/catalog file is required.
   A host offer is routing/availability, **not human consent or agent authority**.

## Prerequisites and limits

- Node 22.18+ (tested 24.15.0), existing standalone package dependencies, and
  supported POSIX process containment. The reversible launcher is
  `beehive/bin/beehive.cjs`; link it once into a user-owned directory already on
  PATH. No reinstall is needed when that link already points to this checkout.
- Private directories/files use 0700/0600. OS credential access uses only the
  Beehive namespace. Creation/readback may synchronously show an OS dialog;
  confirm only when owner-present. Live reads use an owned bounded helper;
  SIGINT/SIGTERM awaits its close, but may not dismiss an OS-owned dialog.
  Missing, denied and unavailable credentials are not automatically regenerated.
- Use the configured private relay (owner-confirmed `wss://buzz.block.builderlab.xyz`).
  Runtime requires NIP42 and private NIP44/59 transport, not NIP11 or NIP43
  advertisements. Actual AUTH, subscription and publication refusals fail as
  transport errors; Beehive does not enroll identities or bypass server policy.
- Configuring does **not** configure provider credentials, create an
  agent, grant initial placement or authorize a conversation channel. Those
  deliberate inputs remain below. Do not use a new genesis to reset an existing
  agent's assignment.
- Historical explicit approval exchange is available only through
  `legacy-enrollment <directory>`; legacy `catalog` input still works. Neither
  is required by normal setup or private discovery.

Automated coverage uses actual setup/host/TUI subprocesses, loopback private transport models, explicit isolated credential adapters, installed Buzz ACP/CLI,
and a deterministic provider fixture. It is **not live Keychain, production
relay, or vendor authentication proof**.

## Optional initial demo agent → signed reply → Stop

Stop the host before changing structural local configuration. Skip this section
if the goal is only to make empty infrastructure available.

1. Prepare a private local `binding.json` (absolute paths, no identity keys):

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
   "agent":"<demo agent hex>", "initialHost":"<configured host hex>" }`.
   This is a locally pinned root, not a signed bearer grant. It must match the
   intended agent and host. **Never replace an existing agent's retained genesis
   or invent a new root to bypass its assignment**; use a separately authorized
   initial demo identity/placement, not migration or Move.

   Run `beehive provision-agent ~/.beehive/host /private/binding.json /private/genesis.json`.
   Enter the matching agent key at the hidden prompt. Expect OS create/readback
   prompts. The owner key is never placed on the host. If interrupted, retain all
   artifacts and use `reconcile-provision` with these exact same inputs, not a new key.
2. Run `beehive auth-info ~/.beehive/host` for the intended service-user
   provider context. Configure the supported provider locally as that user;
   never borrow Desktop credentials. Presence of files is not login proof.
3. Start `beehive host --owner-present`, then open the owner discovery TUI above.
   Use `hosts`, `agents`, select the intended host/agent, `show`, then explicit
   `start`. Wait for an accepted receipt and actual running state; publication
   alone does not prove Start. Mention the authorized demo agent in its usual
   Buzz channel and observe its signed reply.
4. `stop`; wait for accepted receipt and stopped/no actual run. `quit` closes
   only the owner UI. Ctrl-C stops the foreground host and its owned runner.
   Preserve journals and locks if teardown reports uncertainty.

## Remaining deliberate inputs and first-demo limits

The binding file is still necessary: Beehive cannot infer a trusted installed
provider/runtime, allowed workspace, dedicated service HOME/config, conversation
relay, or login consent. Its JSON paths are absolute paths, not shell expressions.
The owner-approved genesis still explicitly pins the intended agent and initial
host: it is not safe to synthesize a replacement root for an existing agent.
This PoC does not yet integrate owner-approved initial placement/provider setup
into a single wizard. Keep those two input files private (0600); host configuration
alone does not authorize an agent or grant conversation channel membership.
No internal revisions or CAS IDs are needed for first Start: the TUI supplies them
from the selected fresh host report. Do not use legacy two-argument setup to bypass
this private configuration path.

After transport loss, owner `reconcile` opens authenticated private transport again.
There is no membership preflight or cached access proof. Actual transport errors
remain errors, never an enrollment workflow or fallback to public management,
owner OA, another signer, or plaintext storage. Registrations, catalogs,
management inventory and lifecycle commands remain private. Move, profile
 distribution and generalized recovery are outside this first demo.
