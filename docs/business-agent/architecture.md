# Architecture

## Boundaries

- Buzz stores the signed question and final answer.
- `buzz-acp` is the trusted Agent Host. It selects the signed source event,
  creates a Turn id, requests a Delegation, creates a fresh ACP Session, and
  injects the token into the MCP child environment.
- `business-auth-gateway` resolves the event author through an active
  `BuzzIdentityBinding` to an active `EnterpriseUser`. It owns issuance,
  atomic call consumption, revocation, expiry, rate limit, and security audit.
- `business-read-mcp` exposes a fixed set of 28 reads and six draft creates, validates inputs,
  consumes one call, invokes the API with service identity, validates the
  response, and emits sanitized audit facts.
- The Business System remains authoritative for roles and data scope. It
  rechecks the current user and domain scope on every call; draft writes also
  use server-derived idempotency keys and the existing domain audit log.
- Business Dock independently uses its existing Business Session. A `biz://`
  link is navigation, never authorization.

## Turn lifecycle

```text
queued -> authorized -> running -> completed | failed | cancelled | timed_out
              |                                  |
              +------- Delegation active --------+
                                                 v
                                              revoked
```

MCP child environment is fixed at ACP `session/new`, so the Host forces a fresh
Session for every Business Turn. A drop guard revokes on every Rust exit path.
Binding or user status is rechecked by every atomic consume, so revocation takes
effect before the next tool call even if best-effort Turn cleanup is delayed.

The dedicated mode drops all ordinary MCP servers and injects only
`business-read-mcp`. Core memory injection is disabled for that runtime.
