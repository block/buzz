# NIP-TG: Relay-Enforced Thread Governance

`buzz.thread-governance/v2` defines a channel-scoped immutable ledger for a
thread's agent-turn budget and owner/lead management decisions. It uses event
kind `42001`.

## Discovery

A supporting relay includes the exact string `buzz.thread-governance/v2` in
the NIP-11 `supported_extensions` array. Clients MUST fail closed for
management actions when the capability is absent. Accepting kind `42001` from
an unadvertised relay is not evidence that the authorization rules below were
enforced.

## Event shape

Every event has exactly one of each of these routing tags:

```json
["h", "<channel UUID>"]
["e", "<64-hex thread root event id>", "", "root"]
["type", "buzz.thread-governance/v2"]
["action", "<action>"]
```

`content` is a JSON object whose `version` member is the number `2`. Clients
define and validate the action-specific content fields. The relay rejects
missing, duplicate, malformed, or unsupported routing tags rather than
guessing whether an event is a management action.

The supported actions are:

- `completed`
- `extend`
- `scope-change`
- `blocker-open`
- `blocker-resolved`
- `closed`
- `defer`

## Relay authorization

`completed` is an ordinary channel write. It passes the channel's normal post
authorization; clients independently verify the completed agent identity when
folding the ledger.

All other actions are management records. For a new management event the relay
MUST, in one database transaction:

1. acquire the same per-channel membership serialization lock used for
   kind `9000` and kind `9001`;
2. read the signer's current active channel role;
3. reject unless that role is `owner` or `admin`; and
4. store the event before releasing the lock.

The signer's `created_at` and any millisecond tag are not authorization
evidence. A record first published after demotion is rejected even if its
signed time predates the demotion. Conversely, a management event already
stored while authorized remains durable and can be returned as a duplicate
after a later role change; historical state is not reauthorized against the
current roster.

Pending local events are not authoritative. A client treats a management
record as accepted only after a strong read from a relay that advertises this
capability proves the record was stored.
