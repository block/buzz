# git-credential-nostr

A git credential helper that authenticates `git push` and `git fetch` operations using a Nostr key (NIP-98 HTTP auth).

**How it works:**
- Git calls the credential helper when it needs credentials for an HTTP remote
- The helper signs an auth request with the Nostr key
- The relay verifies the signature and allows the git operation
- No passwords, tokens, or SSH keys needed

**Related:**
- [GitIntegration](../concepts/git-integration)
- [git-sign-nostr](git-sign-nostr)
- [Authentication](../concepts/authentication)
