# Countdown Bot

A tiny non-AI Sprout bot example.

The bot is deliberately boring and algorithmic: it listens to one Sprout channel
and replies to simple commands:

- `!countdown 5` → `5 4 3 2 1 🚀`
- `!fib 8` → `13 8 5 3 2 1 1 0`
- `@Countdown Bot fib 8` → `13 8 5 3 2 1 1 0`

It demonstrates that Sprout participants do not have to be LLM agents. Any
process that can hold a Nostr key, answer NIP-42 auth, publish a kind `0`
profile, subscribe to events, and publish kind `9` channel messages can be a bot.

On startup it publishes a profile named **Countdown Bot** with a small embedded
SVG clock icon, then best-effort publishes a NIP-29 `kind:9000` self-add with
`role=bot`. That channel membership is what makes the bot show up in the
members list and in Sprout's mention autocomplete.

## Auth paths

### 1. Standalone bot identity

The bot authenticates with its own key only.

Use this when the bot should be admitted as its own independent relay identity.

```bash
SPROUT_RELAY_URL=ws://localhost:3000 \
SPROUT_CHANNEL_ID=<channel-uuid> \
SPROUT_BOT_PRIVATE_KEY=<bot-nsec-or-hex-secret> \
SPROUT_BOT_AUTH_MODE=standalone \
cargo run --manifest-path examples/countdown-bot/Cargo.toml
```

On a closed or allowlisted relay, add the bot pubkey as a relay member or to the
configured pubkey allowlist before starting it. This path does not reuse an
owner's access; revoking the bot requires removing this bot pubkey.

### 2. Owner-attested bot identity

The bot still signs messages with its own key, but its NIP-42 `AUTH` event also
carries a NIP-OA `auth` tag signed by an owner key that is already allowed on the
relay. This reuses the same owner-attestation credential path that Sprout agents
receive after the owner/agent OAuth flow: the relay can let the bot connect
because the owner is a relay member, without making the bot key a persistent
relay member.

Generate the auth tag on the fly:

```bash
SPROUT_RELAY_URL=ws://localhost:3000 \
SPROUT_CHANNEL_ID=<channel-uuid> \
SPROUT_BOT_PRIVATE_KEY=<bot-nsec-or-hex-secret> \
SPROUT_OWNER_PRIVATE_KEY=<owner-or-agent-nsec-or-hex-secret> \
SPROUT_BOT_AUTH_MODE=owner-attested \
cargo run --manifest-path examples/countdown-bot/Cargo.toml
```

Or precompute and pass the tag explicitly:

```bash
SPROUT_AUTH_TAG='["auth","<owner-pubkey>","","<sig>"]' \
SPROUT_BOT_AUTH_MODE=owner-attested \
# plus SPROUT_RELAY_URL, SPROUT_CHANNEL_ID, SPROUT_BOT_PRIVATE_KEY
cargo run --manifest-path examples/countdown-bot/Cargo.toml
```

Relay requirements for this path:

- `SPROUT_REQUIRE_RELAY_MEMBERSHIP=true` on closed relays.
- `SPROUT_ALLOW_NIP_OA_AUTH=true` so owner-attested non-member bot keys can be
  admitted.
- The owner pubkey must be an active relay member.

Relay access and channel access are separate. Owner-attested auth can admit the
bot to the relay, but the bot still publishes as its own pubkey. The bot tries to
self-add to open channels as a `bot` member on startup. For private channels,
an owner/admin must add the bot pubkey to the channel membership before expecting
it to appear in members, resolve in mention autocomplete, or read/write messages.

## Try it locally

1. Start Sprout:

   ```bash
   . ./bin/activate-hermit
   just setup
   just relay
   ```

2. Create or choose a channel in the desktop app and copy its UUID.

3. Run the bot with one of the auth paths above.

4. In the channel, send:

   ```text
   !countdown 5
   !fib 8
   @Countdown Bot fib 8
   ```

## Notes

- Commands are bounded (`!countdown` and `!fib` max 100) so one message cannot
  make the bot spam the relay. Out-of-range commands get an explicit help reply.
- `!fib` replies in descending order because this example is a countdown bot.
- Mention commands require both text like `@Countdown Bot fib 8` and a `p` tag
  for the bot pubkey. The Sprout UI adds that tag when the bot is selected from
  mention autocomplete.
- The bot ignores its own messages to avoid feedback loops.
- The example uses direct WebSocket + NIP-42 instead of MCP so the protocol path
  is easy to inspect in one small file.
