# Buzz agents on Fly.io

This deployment keeps every Buzz agent in a separate Fly app, Machine,
persistent volume, and secret namespace. The `buzz-backend-fly` provider is
discovered by Buzz Desktop and receives the new agent identity only when its
owner presses **Start**.

## One-time operator setup

1. Install and authenticate Fly's official CLI:

   ```bash
   brew install flyctl
   fly auth login
   ```

2. Install `buzz-backend-fly` somewhere Buzz Desktop scans:

   ```bash
   cargo build --release -p buzz-backend-fly
   install -m 0755 target/release/buzz-backend-fly ~/.local/bin/buzz-backend-fly
   ```

3. Build `Dockerfile.fly-agent` and publish it to a registry Fly can pull.
   Give the reviewed build a unique release tag and record the resulting
   manifest digest. Fly CLI 0.4.77 rejects digest references in `machine run`,
   so the pilot provider uses the unique `pilot-20260803` tag while retaining
   the build digest in the deployment record.
   The image contains Sprig plus `mcp-remote@0.1.38`; the package is never
   fetched dynamically when an agent restarts.

4. In Buzz Desktop, create an agent with the **Buzz Agent** runtime, select
   the **fly** backend, and save. The Desktop keeps the Nostr key and owner
   authorization in its existing owner-reviewed flow. Starting the agent
   creates or reconciles exactly one Fly Machine for that public key.

The provider defaults to Amsterdam (`ams`), 1 GB RAM, and a 5 GB encrypted volume
with scheduled snapshots. It creates no public service or IP because the
agent only makes outbound connections to the Buzz relay, model API, and MCP
servers. The Machine uses `on-failure`, so crashes restart while an intentional
clean `!shutdown` stays down. A minimal root entrypoint assigns a newly mounted
volume to the unprivileged `agent` user, then permanently drops privileges
before starting `buzz-acp` or any MCP process.

## Add an MCP connection

Copy `mcp-servers.example.json` to an agent-specific file and use its absolute
path in **MCP profile file**. One profile can declare up to 15 additional
servers when the built-in Buzz developer MCP is enabled.

For a remote server, use the image-pinned `mcp-remote` command. Put account
credentials in the agent's environment and reference their names through
`inherit_env`; do not put the credential value in the shared profile:

```json
[
  {
    "name": "crm",
    "command": "mcp-remote",
    "args": [
      "https://crm.example/mcp",
      "--header",
      "Authorization:${CRM_AUTH_HEADER}",
      "--transport",
      "http-only",
      "--ignore-tool",
      "delete*"
    ],
    "inherit_env": ["CRM_AUTH_HEADER"]
  }
]
```

Connecting the same MCP to two agents means two profiles or grants and two
agent-scoped environment values. Nothing in one Fly app can select another
agent's account. OAuth state created by `mcp-remote` persists under
`/home/agent/.mcp-auth` on that agent's volume.

## Operational checks

```bash
fly status --app <agent-app>
fly logs --app <agent-app>
fly machine list --app <agent-app> --json
fly volumes list --app <agent-app> --json
```

Do not copy the current local agent's `BUZZ_PRIVATE_KEY` into Fly. New cloud
agents must use the separate identity created through Buzz Desktop.
