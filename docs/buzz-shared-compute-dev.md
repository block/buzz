# Buzz shared compute: local GUI verification

This runbook verifies the actual desktop path used by the built-in **Fizz** agent:

`Buzz Desktop → buzz-acp → buzz-agent → MeshLLM SDK → local/remote compute`

It does not use a substitute agent harness.

## Before starting

Run from the `block/buzz` repository root on the mesh-enabled branch.

For a completely fresh, deterministic local state, use:

```bash
. ./bin/activate-hermit
just mesh-dev-fresh
```

This removes development app data, the development keyring entry,
`~/.buzz-dev`, and local Docker volumes; it preserves the installed Buzz app's
data, production keyring, and `~/.buzz`. The first dev page load also clears
only that dev server origin's WebKit storage, so saved fields from an earlier
run cannot leak into the fresh state. It then seeds local channels and starts
the mesh-enabled desktop with the repository's public test identity. That
identity is a fixture and must never be pointed at a shared or production
relay.

If using `mesh-dev-fresh`, the clean window opens at **Welcome to Buzz**. Join
the seeded local community before continuing:

1. Click **Join a community**.
2. Use any local name, such as **Local Buzz**.
3. Set **Community URL** to `ws://localhost:3000` and join.
4. Complete the short profile setup if it appears.

The recipe already supplied the repository's public test identity and seeded
the local channels. Do not import or generate another key. Continue at **Share
this machine** below.

Free the development ports if a previous run was interrupted:

```bash
lsof -nP -iTCP:3000 -iTCP:8080 -iTCP:9102 -iTCP:9337 -iTCP:3131
```

Stop only stale Buzz/MeshLLM processes shown by that command. Do not leave a
standalone `mesh-llm` process using `9337` or `3131`; the desktop owns those
ports during this test.

## 1. Launch the mesh-enabled desktop

```bash
. ./bin/activate-hermit
just mesh=1 dev
```

Keep that terminal open. The first run may build/install the native runtime and
take several minutes. Wait for the Buzz window to open and for the terminal to
stop printing build progress.

Using plain `just dev` is not sufficient: the Compute UI and embedded MeshLLM
runtime are behind the `mesh-llm` feature.

## 2. Share this machine

1. Open **Settings**.
2. Select **Compute**.
3. Under **Share compute**, choose a suggested model.
   - On a 16 GB Apple Silicon machine, use a suggested Qwen3.5 4B quantized
     model when available.
   - `unsloth/Qwen3.5-4B-GGUF:Q4_K_M` is the model used by the hardware proof.
   - Do not use a sub-1B model for the channel-reply proof. It can prove that
     inference is reachable while still failing the agent's long prompt and
     required message-send tool call.
4. Turn on **Share this machine**.
5. Wait until the card says it is sharing/running. Do not start Fizz while the
   card says downloading, preparing, or starting.

Buzz may download the model on first use. The model picker ranks models for the
current hardware; avoid entering a model the card marks too large.

## 3. Make shared compute the agent default

1. Open **Agents** from the left sidebar.
2. In **Agent defaults**, set **Default LLM provider** to
   **Buzz shared compute**.
3. Set **Default model** to **Default (auto)**.
4. Click **Save defaults** and wait for **Saved**.

Fizz has no pinned runtime/provider/model, so it inherits these defaults and
resolves to the bundled `buzz-agent`. No API key is required.

## 4. Start the real Fizz path

1. Find the **Fizz** card on the Agents screen.
2. If Fizz is stopped, click the small play badge over its avatar. If it is
   running, the badge is a green status dot instead of a stop control.
3. Wait for its runtime indicator to become active.
4. Add Fizz to a channel if it is not already a channel member.
5. In that channel, send:

   ```text
   @Fizz Reply exactly: FIZZ_MESH_OK
   ```

6. Confirm that Fizz replies `FIZZ_MESH_OK` in the channel.

That channel response is the end-to-end proof. A green Compute card alone proves
only model serving; it does not prove the Fizz harness and provider inheritance.

To stop a running agent, click the body/name of its card to open its profile,
then click **Stop** near the top. The green avatar badge is status-only while the
agent is running. Once stopped, the profile action becomes **Respawn** and the
avatar badge becomes a play button.

To create a separate test agent, choose **New agent → New agent**, use
**buzz-agent** as the runtime, **Buzz shared compute** as the LLM provider,
**Default (auto)** as the model, and **This computer** under **Run on**. Shared
compute is an LLM provider; do not select a remote compute backend as the run
location merely because its name mentions mesh.

## 5. Optional diagnostics

While Buzz is running:

```bash
# The desktop should own both ports.
lsof -nP -iTCP:9337 -iTCP:3131

# The embedded OpenAI-compatible ingress should advertise the model.
curl -sS http://127.0.0.1:9337/v1/models | jq '.data[].id'

# Fizz should resolve through the real managed-agent subprocesses.
ps -eo pid,ppid,command | grep -E '[b]uzz-(desktop|acp|agent)'
```

If Fizz fails, open its runtime details from the Agents screen first. Common
causes are:

- launched with `just dev` instead of `just mesh=1 dev`;
- a stale process owns `9337`/`3131`;
- the model is still downloading or preparing;
- Fizz is not a member of the channel;
- defaults were changed but not saved;
- no current Buzz membership snapshot is available (admission fails closed).

## Security boundary

Buzz publishes member-signed discovery notes through an ordinary relay-supported
NIP-51 event. The note includes a MeshLLM-key signature binding the member to the
advertised MeshLLM node identity, plus a second signature over the exact endpoint
tokens in the note. Current Buzz membership controls which node identities are
admitted. A serving target is selectable only when its endpoint signature is
valid, its invite token decodes as a bounded Iroh endpoint, and every advertised
relay URL matches this machine's locally configured Iroh relay policy.

`BUZZ_MESH_IROH_RELAYS` defaults to Iroh's production relay set. Set it to `0`
for direct QUIC only, or to a comma-separated HTTPS allowlist for custom relays.
Plain HTTP is accepted only for loopback development relays. Remote status notes
cannot expand this local allowlist.

MeshLLM—not the Buzz relay—carries inference over direct QUIC or its encrypted
iroh relays and enforces the owner allowlist. The dependency is pinned to the
post-v0.72.2 admission fix that prevents a non-member with a leaked invite token
from using passive inference streams. MeshLLM v0.73.1 still performs its owner
check during gossip after transport connection; authenticating before any gossip
is an upstream protocol change and is not claimed by the Buzz-side checks above.
