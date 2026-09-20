# Local forward input

On Unix, `--forward-socket PATH --forward-peer-uid UID --forward-state PATH`
enables a length-prefixed JSON input compatible with the Hermes Buzz gateway.
The state path is required explicitly; use a private persistent directory.
`--relay-input false` disables channel and observer input subscriptions while
retaining membership updates and outbound publication. It requires a forward
socket. Setup mode retains its existing relay input.

The socket checks native peer UID, original Nostr event signature, channel tag,
canonical channel UUID, membership and the SHA-256 dedupe key. Membership and
observer-control events are not admitted over the socket. Normal events use
the same author gate, mention/subscription rules, self filter, thread scope,
queue and steering path as relay events. Forward events refresh relay signing
identity before delegated workflow attribution because they have no WebSocket
connection generation. Windows can continue using relay input; forward socket
configuration is rejected there.

One connection sends a four-byte big-endian length, then a JSON object with
`event`, `direction: "inbound"`, `chat_type`, `channel_id`, and `dedupe_key`.
The key is SHA-256 over event ID hex + `inbound` + canonical channel UUID.
Frames are limited to 8 MiB, with read deadlines and 32 concurrent connections.
ACKs use the same length prefix and contain `ack` (the event ID), `status`,
and `reason` (a string or null). The socket uses mode 0660: the client must
also have filesystem access through ownership/group permissions; passing the
peer UID check alone does not grant that access.

`accepted` means admission or intentional policy filtering plus a persisted
occupancy key. `duplicate` also covers the configured drop-while-busy policy.
`rejected` is terminal for the current Hermes producer, including membership
mismatch. Keep both sides' channel sets synchronized. Infrastructure failure
has no ACK and invites retry.

The queue is in memory: ACK is **not** a completed turn or durable work queue.
A crash after admission/persist can lose queued work; a crash before persist
can permit replay. The append-only occupancy file reloads its most recent
50,000 lines; older IDs may replay after restart. The append file and runtime
reservation set grow during a process lifetime; budget storage and memory. A concurrent duplicate can
persist while the original admission is still pending. These bounds make this
an opt-in local adapter, not exactly-once delivery. Use one consumer process
per socket/state file and protect its parent directory from untrusted writers.
