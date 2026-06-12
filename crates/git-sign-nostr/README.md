# git-sign-nostr

NIP-GS signing program — signs git commits and tags with Nostr secp256k1 keys
using BIP-340 Schnorr signatures.

## Usage

```bash
# Configure git to use nostr signing
git config gpg.format x509
git config gpg.x509.program /path/to/git-sign-nostr
git config commit.gpgsign true
git config tag.gpgsign true
git config user.signingkey <hex-pubkey>

# Set the private key (env var)
export NOSTR_PRIVATE_KEY=<hex-or-nsec>

# Optional: NIP-OA owner attestation
export BUZZ_AUTH_TAG='["auth","<owner-pk>","<conditions>","<owner-sig>"]'

# Commits are now automatically signed
git commit -m "signed with nostr"

# Verify
git verify-commit HEAD
```

## Key Loading Priority

1. `NOSTR_PRIVATE_KEY` environment variable
2. `BUZZ_PRIVATE_KEY` environment variable
3. Keyfile at path from `git config nostr.keyfile`

Keys may be hex (64 chars) or NIP-19 bech32 (`nsec1...`).

## How It Works

Git invokes this program as a signing/verification backend:

- **Sign:** `git-sign-nostr --status-fd=2 -bsau <keyid>` — reads payload from
  stdin, writes armored signature to stdout, status lines to fd 2 (stderr)
- **Verify:** `git-sign-nostr --status-fd=1 --verify <sigfile> -` — reads
  payload from stdin, verifies signature from file, status lines to fd 1 (stdout)

See [NIP-GS](../../docs/nips/NIP-GS.md) for the full specification.
