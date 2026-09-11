# Local broker prototype

One process holds the Buzz key and one `IfcSession`. A keyless client can read
recent messages from one channel or post plain text there. The operator fixes
the relay, channel, community, and requester at startup. Requests cannot select
another destination, sign an arbitrary event, or change the session.

This is an opt-in CLI prototype, not a hardened local security boundary. It does
not change how Desktop or `buzz-acp` launches agents; those paths still supply
credentials as before. It does not implement the SDK's full HTTP broker contract.

## Run it

Build with `cargo build -p buzz-cli`. In an operator terminal that already has
the agent's `BUZZ_PRIVATE_KEY` (and `BUZZ_AUTH_TAG`, if needed), start:

```sh
buzz --relay https://your-community.example broker serve \
  --community <community-uuid> \
  --channel <channel-uuid> \
  --relay-key <relay-signing-public-key> \
  --requester <requesting-person-public-key>
```

Get the community UUID and relay signing key from a trusted operator, not from
the agent. Both the agent and requester must be current channel members. Only
public or private stream channels are supported; DMs, forums, thread routing,
mentions, media, and arbitrary event kinds are deliberately absent.

The foreground process prints `{"socket":"...","channel":"..."}`. Give only
that socket path to a **fresh** agent with no Buzz credential environment:

```sh
buzz broker read --socket <socket-path> --limit 20
buzz broker reply --socket <socket-path> --content 'Plain text answer'
```

`reply` posts a top-level message in the fixed channel. It does not resolve
mentions or expand text into additional operations. The read limit is 1–100;
reply text is at most 16 KiB. The client does not connect directly to the relay
or fall back to a private key when the socket is unavailable.

Stop the broker with Ctrl-C. Each startup creates a new socket in a private
temporary directory. Do not reconnect an agent's old history or files to a new
broker: it would discard the old IFC state. This prototype does not manage that
agent lifetime for you.

## Checks and limits

The broker verifies relay signatures on metadata and the complete membership
snapshot before creating the domain, before each operation, and again before
delivering a read. It validates each returned message's signature, kind, and
channel. A changed snapshot or failed policy check permanently blocks new
publications from that session, including after a policy rollback. Even a
metadata edit or temporary policy-query failure requires a fresh agent and
broker before writing again; there is no recovery API that clears IFC state.

The broker builds the outgoing event itself, passes its serialized bytes to
`IfcSession::publish`, and sends exactly those bytes. It never automatically
retries a publication. If the connection fails, the message may already have
been accepted; inspect the channel before deciding to send again.

Membership checks and publication are **not atomic**. Membership may change
after the final check but before the relay accepts a message. The relay remains
trusted for current access checks, and a future relay-side policy-version
precondition is needed to close that race. This is not a claim of complete IFC
enforcement under concurrent membership changes.

The socket's directory is mode 0700 and the socket is 0600. That excludes other
OS users, not another process running as the same user. Such a process may read
the broker's environment or memory, replace its executable, or use its socket.
There is no signed helper, Hardened Runtime enforcement, or protected Keychain
storage here. Do not treat this prototype as protection against a hostile local
agent. It also cannot label the agent's filesystem, model history, other tools,
or network traffic: start with fresh state and do not feed it unrelated private
data. End-to-end confidentiality requires those boundaries too.

Tests live beside the code, with each invariant explained inline. Run
`cargo test -p buzz-cli broker` for the socket/HTTP tests against a signed mock
relay; they do not require a running Buzz deployment.

To also exercise separate broker/client processes and shutdown on macOS/Unix:

```sh
cargo build -p buzz-cli
BUZZ_TEST_BROKER_BIN="$PWD/target/debug/buzz" \
  cargo test -p buzz-cli broker_executable_smoke -- --ignored
```
