# OpenClaw provider enrollment

Buzz Desktop exposes remote run locations through discovered executables named
`buzz-backend-<id>`. An OpenClaw integration should install
`buzz-backend-openclaw` on the Desktop machine; it will then appear under
**Run on** without Buzz Desktop becoming a Gateway or runtime proxy.

## Provider contract

The provider is a one-process-per-operation stdin/stdout JSON executable. It
must answer `info` first, with protocol version `1`:

```json
{
  "ok": true,
  "name": "openclaw",
  "version": "1.0.0",
  "protocol_version": 1,
  "description": "Enrolls agents with an OpenClaw host",
  "config_schema": {
    "type": "object",
    "properties": {
      "host": { "type": "string", "description": "Optional SSH destination (advanced fallback)" },
      "rooms": { "type": "string", "description": "Comma-separated Buzz room UUIDs" },
      "port": { "type": "string", "description": "Optional SSH port" }
    },
    "required": ["rooms"]
  },
  "enrollment": {
    "operation": "enroll",
    "one_time": true,
    "credential_fields": ["private_key_nsec", "auth_tag", "relay_url"]
  }
}
```

`config_schema` is persisted in the agent record and is therefore not a place
for credentials. Desktop rejects secret-shaped keys (`key`, `token`,
`credential`, `password`, `secret`) and nested values in provider config.
Host authentication belongs to the provider's normal local trust mechanism
(for example, an existing OpenClaw CLI login or OS credential store).

After `info`, Desktop invokes the same staged provider binary once with:

```json
{
  "op": "enroll",
  "request_id": "uuid",
  "agent": {
    "name": "display name",
    "relay_url": "wss://relay.example",
    "private_key_nsec": "nsec1...",
    "auth_tag": "[\"auth\",\"owner\",\"\",\"signature\"]",
    "respond_to": "owner-only",
    "respond_to_allowlist": [],
    "env_vars": {},
    "launch": {
      "command": "openclaw",
      "args": ["acp"],
      "env": {},
      "policy_env": {},
      "owner_pubkey": "hex"
    }
  },
  "provider_config": {
    "host": "openclaw@agent-host",
    "rooms": "ROOM_UUID_1,ROOM_UUID_2",
    "port": "22"
  },
  "enrollment": { "version": 1, "mode": "one-time" }
}
```

The exact managed-agent payload is the source of truth; providers must not
reconstruct identity from `env_vars`. Without `host`, the provider generates
a signed, short-lived code and returns a copyable command for the Desktop
operator:

```bash
openclaw buzz enroll --code 'buzz-enroll-v1....'
```

With `host`, the provider preserves the legacy SSH handoff and runs
`openclaw buzz enroll --stdin` on the remote host.
It imports the identity and room configuration into OpenClaw and returns a
stable host-side identifier:

```json
{ "ok": true, "agent_id": "openclaw-host-agent-id" }
```

Desktop stores only that identifier and the non-secret provider config. It
does not retain a host session, proxy messages, poll the Gateway, or provide
remote logs/stop controls. Subsequent conversation and presence continue over
the Buzz relay. Re-running the action must be idempotent for the same agent
identity; the provider owns reconciliation on the OpenClaw host.
The community relay does not need Tailscale: OpenClaw connects outbound to it
directly.

On any failure, return `{"ok":false,"error":"..."}` and exit zero. Exit
non-zero means the response is untrusted. Provider diagnostics must never
echo the nsec, auth tag, or environment secrets.
