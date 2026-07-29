# git-sign-nostr

A tool for signing git objects (commits, tags) with a Nostr key rather than a traditional PGP key.

**Usage:** Replaces or supplements `git commit -S` / `git tag -s`. The signature is generated using the Nostr keypair (secp256k1) instead of a PGP key.

**Related:**
- [GitIntegration](../concepts/git-integration)
- [git-credential-nostr](git-credential-nostr)
