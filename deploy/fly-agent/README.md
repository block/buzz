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
   so the pilot provider uses the unique
   `pilot-20260804-project-mcp-boundary` tag while retaining the build digest
   in the deployment record.
   The image contains Sprig and does not fetch runtime packages when an agent
   restarts.

4. In Buzz Desktop, create an agent with the **Buzz Agent** runtime, select
   the **fly** backend, and save. The Desktop keeps the Nostr key and owner
   authorization in its existing owner-reviewed flow. Starting the agent
   creates or reconciles exactly one Fly Machine for that public key.

The provider defaults to Amsterdam (`ams`), 1 GB RAM, and a 5 GB encrypted volume
with scheduled snapshots. It creates no public service or IP because the
agent only makes outbound connections to the Buzz relay and model API. The
Machine uses `on-failure`, so crashes restart while an intentional
clean `!shutdown` stays down. A minimal root entrypoint assigns a newly mounted
volume to the unprivileged `agent` user, then permanently drops privileges
before starting `buzz-acp` or any child process.

## Project connection boundary

This provider intentionally does not accept agent-owned MCP endpoints or
credentials. Project configuration owns the concrete connection, secret,
health state, and discovered tools; an agent owns only a connection binding and
optional tool allowlist. Those bindings are being developed in #4588 on the
shared MCP schema from #4164 and the HTTP transport work in #4271.

Fly agents cannot execute a Desktop-local stdio connection. A future Project
binding must therefore resolve to a cloud-reachable HTTP connection or a
gateway. At deploy time, the control plane may materialize a scoped runtime
copy of the Project credential into that Fly app's secret boundary, but the
credential must never be serialized into the agent, persona, template, or
snapshot.

## Operational checks

```bash
fly status --app <agent-app>
fly logs --app <agent-app>
fly machine list --app <agent-app> --json
fly volumes list --app <agent-app> --json
```

Do not copy the current local agent's `BUZZ_PRIVATE_KEY` into Fly. New cloud
agents must use the separate identity created through Buzz Desktop.
