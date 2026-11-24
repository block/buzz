# git-credential-nostr

NIP-98 credential helper for git — signs HTTP auth events with your Nostr key so git can push/pull from Buzz's git server without passwords.

## Requirements

- **git 2.46+** (requires `authtype` capability in the credential protocol)
- **Rust toolchain** (for building from source)

## Installation

```bash
cargo install --path crates/git-credential-nostr
```

## Setup

```bash
# 1. Register the helper and enable per-path credentials.
git config --global credential.helper nostr
git config --global credential.useHttpPath true

# 2. Store your nsec in a key file (must be 0600).
mkdir -p ~/.nostr
echo "nsec1..." > ~/.nostr/key && chmod 600 ~/.nostr/key
git config --global nostr.keyfile ~/.nostr/key
```

That's it. Use git normally — `git clone`, `git push`, `git fetch`.

## CI / CD

Set `$NOSTR_PRIVATE_KEY` instead of a key file. The env var takes precedence
over `nostr.keyfile` and avoids touching the filesystem:

```bash
export NOSTR_PRIVATE_KEY=nsec1...
git clone https://relay.example.com/git/owner/repo.git
```

## Agents

An agent proves its owner with a NIP-OA tag that the helper signs into each
request. It reads the tag from `$BUZZ_AUTH_TAG`, else from git config
`nostr.authtag`. An agent in the repo's channel needs nothing more: its own
role there decides. An agent outside it can act under a Git grant from its
owner, a tag naming the repo and level, capped by the owner's own role in the
repo's channel. The owner mints one with `buzz repos grant`, signing with the
owner's key, and the agent passes it to the command it is for:

```bash
BUZZ_AUTH_TAG="$(buzz repos grant --agent <agent-hex> --repo 30617:<owner-hex>:<repo> --git push --for 2h)" \
  git push origin feature
```

A grant is good only for the window it states. To stop it early, an admin
bans the agent's key; a ban on the owner stops it too.

## How It Works

When a Buzz git server returns `HTTP 401` with a
`WWW-Authenticate: Nostr realm="...", method="GET"` header, git calls this
helper with the request details on stdin. The helper loads your Nostr private
key, builds a [NIP-98](https://github.com/nostr-protocol/nips/blob/master/98.md)
kind-27235 event signed over the request URL and method, base64-encodes it, and
writes it back to stdout. Git then retries the request with
`Authorization: Nostr <token>`, which the server verifies by checking the event
signature.

```
git ──stdin──▶ git-credential-nostr ──stdout──▶ git
                     │
                     ▼
              sign kind:27235 event
              (NIP-98 HTTP Auth)
```

## Troubleshooting

| Error | Cause | Fix |
|-------|-------|-----|
| `no nostr key configured` | Neither `$NOSTR_PRIVATE_KEY` nor `nostr.keyfile` is set | Follow the Setup steps above |
| `insecure permissions` | Key file is readable by group/others | `chmod 600 ~/.nostr/key` |
| `method hint` | Server's `WWW-Authenticate` header is missing `method="..."` | Upgrade the Buzz server |
| `useHttpPath` | `credential.useHttpPath` is not set | `git config --global credential.useHttpPath true` |
| Empty output / no auth | git version is older than 2.46 | Upgrade git |
| `clock skew` / auth rejected | System clock is off by more than 60 s | Sync your system clock (`ntpdate`, `timedatectl`) |
