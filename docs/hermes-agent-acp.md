# Run a Hermes Agent in Buzz

Buzz can host an existing [Hermes Agent](https://github.com/NousResearch/hermes-agent)
through Agent Client Protocol (ACP). Hermes remains the agent runtime and keeps
its provider configuration, credentials, memory, skills, and tools. `buzz-acp`
owns the Buzz connection, channel subscriptions, and message delivery.

```text
Buzz relay <-- WebSocket --> buzz-acp <-- ACP over stdio --> Hermes Agent
                                |
                                +-- injects Buzz context and CLI environment
```

This guide covers a persistent Linux deployment. For a local desktop setup,
choose **Hermes Agent** in Buzz Desktop after it detects `hermes-acp`. A custom
command with `hermes` and the argument `acp` is equivalent.

## Prerequisites

- A configured Hermes Agent installation on the host.
- The Hermes ACP extra and either `hermes acp` or `hermes-acp` on `PATH`.
- A `buzz-acp` binary built from this repository.
- A dedicated Buzz/Nostr identity for the agent.
- Membership for that identity in at least one Buzz channel.

Verify both runtimes before adding credentials:

```bash
hermes acp --version
hermes acp --check
buzz-acp --version
```

If Hermes ACP is not installed:

```bash
cd ~/.hermes/hermes-agent
uv pip install -e '.[acp]'
```

See the
[Hermes ACP guide](https://hermes-agent.nousresearch.com/docs/user-guide/features/acp/)
for the supported installation and provider setup.

Build the Buzz harness:

```bash
git clone https://github.com/block/buzz.git
cd buzz
. ./bin/activate-hermit
cargo build --release -p buzz-acp -p buzz-cli
```

Install `target/release/buzz-acp` and `target/release/buzz` somewhere on the
service account's `PATH`.

## Provision a dedicated Buzz identity

Do not reuse a human identity or another agent's private key. Each agent should
have its own public identity so membership, attribution, and revocation remain
independent.

For a self-hosted relay, mint credentials with:

```bash
cargo run -p buzz-admin -- mint-token \
  --name "hermes-agent" \
  --scopes "messages:read,messages:write,channels:read"
```

For a hosted community, ask the community owner to provision the agent
identity. Keep the returned private key and API token on the agent host. Share
only the public key when adding the agent to channels.

Add the public key to each intended channel with the `bot` role:

```bash
buzz channels add-member \
  --channel <channel-uuid> \
  --pubkey <agent-public-key-hex> \
  --role bot
```

Channel membership is the access boundary. On startup, `buzz-acp` discovers
every channel where the agent is a member. It also subscribes automatically
when the agent is added to another channel, so the service does not need a
restart for ordinary membership changes.

After discovery, the harness publishes the identity's complete kind `10100`
agent-directory profile. Buzz Desktop uses that record to show externally
hosted agents under **Agents → External agents** and to decide whether they
belong in the `@` mention picker. The profile carries the identity's kind `0`
display name, channel list, verified NIP-OA owner, and inbound author gate; it
contains no credentials.

## Configure the bridge

Create a credential file readable only by the service account. The values
below are examples; do not commit the real file.

```bash
BUZZ_RELAY_URL=wss://community.example.com
BUZZ_PRIVATE_KEY=nsec1...
BUZZ_API_TOKEN=...
BUZZ_AUTH_TAG=...

BUZZ_ACP_AGENT_COMMAND=/home/hermes/.local/bin/hermes
BUZZ_ACP_AGENT_ARGS=acp
BUZZ_ACP_RESPOND_TO=allowlist
BUZZ_ACP_RESPOND_TO_ALLOWLIST=<trusted-user-pubkey-hex>
```

`BUZZ_API_TOKEN` is needed only when the relay enforces token authentication.
`BUZZ_AUTH_TAG` is needed for owner-authorized operations such as opening
agent drafts; normal channel messaging does not require it.

Choose the inbound author gate deliberately:

| Value | Behavior |
|---|---|
| `owner-only` | Only the registered owner can prompt the agent. This is the default. |
| `allowlist` | The owner and the listed public keys can prompt the agent. |
| `anyone` | Any channel member can prompt the agent. Use only for a deliberately open agent. |
| `nobody` | Ignore inbound prompts; useful only with proactive heartbeat work. |

The default ACP permission mode is `bypassPermissions` because a headless
message bridge cannot stop at an editor approval dialog. This gives Hermes the
same host capabilities it has when run non-interactively. Use a dedicated
operating-system account, limit its filesystem permissions, and restrict the
inbound author gate. Set `BUZZ_ACP_PERMISSION_MODE=default` only when the ACP
client and your operational workflow can service permission requests.

## Run under systemd

The service must run as the same operating-system user that owns the intended
Hermes home. Otherwise Hermes will load a different configuration and memory.

Create `/etc/systemd/system/buzz-hermes.service`:

```ini
[Unit]
Description=Buzz ACP bridge for Hermes Agent
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=hermes
Group=hermes
Environment=HOME=/home/hermes
EnvironmentFile=/etc/buzz-hermes/bridge.env
ExecStart=/usr/local/bin/buzz-acp
Restart=always
RestartSec=5
NoNewPrivileges=true
PrivateTmp=true

[Install]
WantedBy=multi-user.target
```

Protect the environment file and start the service:

```bash
sudo install -d -o root -g hermes -m 0750 /etc/buzz-hermes
sudo chown root:hermes /etc/buzz-hermes/bridge.env
sudo chmod 0640 /etc/buzz-hermes/bridge.env
sudo systemctl daemon-reload
sudo systemctl enable --now buzz-hermes.service
```

Use `0600` instead when the service itself runs as `root`. Running Hermes as a
dedicated unprivileged user is preferred.

## Verify the live integration

Check local health without printing secrets:

```bash
systemctl is-enabled buzz-hermes.service
systemctl is-active buzz-hermes.service
systemctl show buzz-hermes.service -p Restart -p User -p EnvironmentFiles
journalctl -u buzz-hermes.service -n 100 --no-pager
```

The startup log should show:

- the expected relay URL and Hermes command;
- a successful ACP adapter start;
- the expected Hermes tools and memory mode;
- at least one discovered channel.
- `published agent directory profile`.

Then run a real round trip:

1. Open **Agents → External agents** and confirm the identity appears by name.
2. Type `@<agent-name>` in a channel where it has the `bot` role and select it
   from the composer picker.
3. Ask it to identify its runtime and reply in the same thread.
4. Confirm the reply is authored by the dedicated agent public key.
5. Confirm the reply event is accepted by the relay.

This validates the entire path, not just process health:

```text
Buzz event -> relay subscription -> buzz-acp -> Hermes ACP turn
          -> Buzz CLI reply -> signed relay event -> channel thread
```

## Operational behavior

- Hermes continues to read its normal home, including `config.yaml`, `.env`,
  skills, memory, and state database.
- `buzz-acp` injects the Buzz base instructions and authenticated `buzz` CLI
  environment into the ACP subprocess.
- One Hermes ACP session is maintained per Buzz channel.
- New mentions are queued per channel; separate channels can run concurrently
  when `BUZZ_ACP_AGENTS` is greater than one.
- Relay disconnects are retried, and channel subscriptions resume with replay
  protection.
- `Restart=always` restarts the bridge after a process or host failure.

## Troubleshooting

### Hermes exits during ACP initialization

Run the same command as the service account:

```bash
sudo -u hermes -H /home/hermes/.local/bin/hermes acp --check
```

If that fails, fix the Hermes ACP installation or provider configuration before
debugging Buzz.

### The service is active but the agent does not respond

Check:

- the agent public key has the `bot` role in the channel;
- the message mentions the agent's exact Buzz identity;
- the author passes `BUZZ_ACP_RESPOND_TO`;
- the service account loads the intended Hermes home;
- the relay URL uses `ws://` or `wss://`, not `http://` or `https://`.

### Hermes can answer but cannot post

Confirm `buzz` is on the service `PATH` and the private key, API token, and
optional owner authorization belong to the same agent identity.

### A new channel is not discovered

Verify membership from an owner account:

```bash
buzz channels members --channel <channel-uuid>
```

The agent should appear with role `bot`. Membership notifications normally add
the subscription without a restart; restart once if the bridge was offline
when the membership event was issued.

## Security checklist

- Use one dedicated Buzz identity per agent.
- Never paste or log `BUZZ_PRIVATE_KEY`, `BUZZ_API_TOKEN`, or `BUZZ_AUTH_TAG`.
- Store credentials outside the repository with restrictive permissions.
- Run Hermes under a dedicated unprivileged operating-system account.
- Prefer `owner-only` or `allowlist` over `anyone`.
- Grant channel membership only where the agent is expected to work.
- Review Hermes tool access before using `bypassPermissions`.
- Rotate and revoke the dedicated identity independently if the host is
  compromised.
