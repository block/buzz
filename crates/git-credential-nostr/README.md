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

### macOS: scope the helper to your relay

On macOS, Xcode's bundled gitconfig registers `osxkeychain` as a credential
helper for every repository:

```
$ git config --show-origin --get-all credential.helper
file:/Applications/Xcode.app/Contents/Developer/usr/share/git-core/gitconfig   osxkeychain
```

`git config --global credential.helper nostr` **appends** to that chain rather
than replacing it. Both helpers run, and after each successful operation git
asks every helper in the chain to store the credential — osxkeychain cannot
store the Nostr credential shape and prints:

```
failed to store: -1
```

Operations still succeed; the error is pure noise. To silence it — and to keep
the helper from ever running for github.com or other hosts — register the
helper scoped to your relay URL instead of globally (the same idiom GitHub's
`gh` CLI uses):

```bash
git config --global --add credential.https://relay.example.com.helper ""     # reset the chain for this URL
git config --global --add credential.https://relay.example.com.helper nostr
git config --global credential.useHttpPath true
```

The empty first entry resets the helper chain for that URL, so only `nostr`
runs against your relay and osxkeychain keeps handling everything else.

## CI / CD

Set `$NOSTR_PRIVATE_KEY` instead of a key file. The env var takes precedence
over `nostr.keyfile` and avoids touching the filesystem:

```bash
export NOSTR_PRIVATE_KEY=nsec1...
git clone https://relay.example.com/git/owner/repo.git
```

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
| `failed to store: -1` after successful operations (macOS) | Xcode's global `osxkeychain` helper runs alongside `nostr` and can't store the Nostr credential | Scope the helper to your relay URL — see [macOS](#macos-scope-the-helper-to-your-relay) |
| `clock skew` / auth rejected | System clock is off by more than 60 s | Sync your system clock (`ntpdate`, `timedatectl`) |
