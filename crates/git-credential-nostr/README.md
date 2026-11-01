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

# 2. Store your nsec through the interactive daz-secrets CLI.
daz-secrets set buzz-desktop identity
git config --global nostr.secretService buzz-desktop
git config --global nostr.secretAccount identity
```

That's it. Use git normally — `git clone`, `git push`, `git fetch`.

## CI / CD

Install an unattended daz-secrets provider for the CI account, then configure
only the nonsecret provider coordinates:

```bash
git config nostr.secretService ci-build
git config nostr.secretAccount relay-identity
git clone https://relay.example.com/git/owner/repo.git
```

## How It Works

When a Buzz git server returns `HTTP 401` with a
`WWW-Authenticate: Nostr realm="...", method="GET"` header, git calls this
helper with the request details on stdin. The helper reads your Nostr private
key directly from daz-secrets, builds a [NIP-98](https://github.com/nostr-protocol/nips/blob/master/98.md)
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
| `nostr identity is unavailable from daz-secrets` | The configured provider item is absent or unavailable | Verify the provider and `nostr.secretService` / `nostr.secretAccount` |
| `method hint` | Server's `WWW-Authenticate` header is missing `method="..."` | Upgrade the Buzz server |
| `useHttpPath` | `credential.useHttpPath` is not set | `git config --global credential.useHttpPath true` |
| Empty output / no auth | git version is older than 2.46 | Upgrade git |
| `clock skew` / auth rejected | System clock is off by more than 60 s | Sync your system clock (`ntpdate`, `timedatectl`) |
