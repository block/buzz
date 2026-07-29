# Git Integration

Buzz implements NIP-34 for git hosting, plus additional tools for Nostr-signed git operations.

## NIP-34 Git Hosting

The relay hosts bare git repos over smart HTTP. Repos are accessed via the relay's HTTP endpoints, authenticated with NIP-98.

## Branch-as-Room

Creating a feature branch automatically creates a channel where patches, CI results, reviews, and merge decisions live. This ties git workflow directly into the chat workspace.

## Tools

- **git-sign-nostr** — sign git commits, tags, and other objects with a Nostr key
- **git-credential-nostr** — git credential helper that authenticates push/fetch using Nostr keys

**Related:**
- [git-sign-nostr](../components/git-sign-nostr)
- [git-credential-nostr](../components/git-credential-nostr)
- [NostrProtocol](nostr-protocol)
