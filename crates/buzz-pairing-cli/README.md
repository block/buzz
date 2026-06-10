# sprout-pair

CLI tool for testing the [NIP-AB device pairing protocol](../sprout-core/src/pairing/NIP-AB.md) end-to-end. Exercises the full protocol over a live Nostr relay — designed for interop testing and NIP submission, not production use.

## Quick Start

```bash
cargo build --release -p sprout-pairing-cli

# Terminal 1 — source (holds the secret)
./target/release/sprout-pair source --relay wss://relay.damus.io

# Terminal 2 — target (receives the secret)
./target/release/sprout-pair target --show-secret
# paste the QR URI from terminal 1 when prompted
```

Both sides display a 6-digit SAS code. Confirm they match on each side, and the key transfers.

## Subcommands

### `source`

Acts as the device holding the secret. Generates an ephemeral keypair and session secret, displays a `nostrpair://` QR URI, waits for a target to connect, performs SAS verification, and sends the payload.

```
sprout-pair source --relay <RELAY_URL> [--nsec <BECH32_NSEC>]
```

- `--relay` — WebSocket relay URL (default: `wss://relay.damus.io`)
- `--nsec` — bech32 nsec to transfer. If omitted, generates a throwaway test key.

### `target`

Acts as the receiving device. Reads a `nostrpair://` URI from stdin, connects to the relay encoded in the URI, sends an offer, verifies SAS, and receives the payload.

```
sprout-pair target [--relay <OVERRIDE_URL>] [--show-secret]
```

- `--relay` — Override the relay URL from the QR code
- `--show-secret` — Print the received secret to stdout (off by default for safety)

### `test-vectors`

Prints all derived cryptographic values from the NIP-AB spec's fixed test keys. Useful for verifying implementations against the spec.

```
sprout-pair test-vectors
```

## Testing Against a Local Sprout Relay

The CLI supports NIP-42 authentication, so it works with Sprout relays out of the box.

### Prerequisites

- Docker running (for Postgres, Redis, etc.)
- Sprout relay built: `cargo build --release -p sprout-relay`

### Start the relay

```bash
just setup                          # Docker services + schema
cargo build --release --workspace
screen -dmS relay bash -c "./target/release/sprout-relay 2>&1 | tee /tmp/sprout-relay.log"
sleep 3 && curl -s http://localhost:3000/health   # → "ok"
```

### Run the E2E test

An automated test script using `expect` is provided:

```bash
.scratch/e2e-pair-local.sh
```

This spawns source and target as PTY-driven subprocesses, feeds the QR URI between them, waits for both SAS codes to appear, delays to ensure relay subscriptions are registered, then confirms SAS on both sides. Prints `PASS` or `FAIL` with the SAS codes.

**Requirements:** `expect` (macOS: built-in at `/usr/bin/expect`)

**Environment variables:**

| Variable | Default | Description |
|----------|---------|-------------|
| `RELAY_URL` | `ws://localhost:3000` | Relay to test against |
| `TEST_TIMEOUT` | `45` | Per-step timeout in seconds |
| `SOURCE_CONFIRM_DELAY_MS` | `3000` | Delay after SAS display before confirming (lets relay register subscriptions) |

### Manual two-terminal test

```bash
# Terminal 1
./target/release/sprout-pair source --relay ws://localhost:3000

# Terminal 2
./target/release/sprout-pair target --show-secret
# paste the nostrpair:// URI, confirm SAS on both sides
```

## Protocol Overview

```
Source                          Relay                    Target
──────                          ─────                    ──────
Generate ephemeral keys
Display QR (pubkey+secret+relay)
Subscribe kind:24134                                     Scan QR
                                                         Generate ephemeral keys
                                                         Subscribe kind:24134
                                                         Wait for EOSE
                                ◄─────────────────────── Send offer
Verify session_id
Compute SAS ◄──────────────────────────────────────────► Compute SAS
Display: "047291"                                        Display: "047291"

[User confirms codes match]

Send sas-confirm ──────────────►─────────────────────►
                                                         Verify transcript_hash
                                                         [User confirms]
Send payload ──────────────────►─────────────────────►
                                                         Decrypt + import
                                ◄─────────────────────── Send complete
Done                                                     Done
```

All events are NIP-44 encrypted, signed with ephemeral keys, and addressed via `p` tags. The relay sees only opaque ciphertext between throwaway pubkeys.
