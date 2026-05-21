# sprout-cli Live Testing Guide

Manual testing runbook for verifying every CLI command against a local relay.
An agent or developer follows this step by step, running each command and
checking the output.

---

## 1. Prerequisites

Docker services running and healthy:

```bash
docker compose ps
# sprout-postgres   healthy
# sprout-redis      healthy
# sprout-typesense  healthy
```

If not running: `./scripts/dev-setup.sh` from the repo root.

Tools: `jq`, `curl`, Rust toolchain.

---

## 2. Build the CLI

```bash
cargo build -p sprout-cli
```

Use `cargo run -p sprout-cli --` or the built binary at `target/debug/sprout`.

---

## 3. Start the Relay

In a separate terminal:

```bash
cd REPOS/sprout-nostr
set -a && source .env && set +a
cargo run -p sprout-relay
```

Verify:

```bash
curl -s http://localhost:3000/_liveness
# "ok" or 200 status
```

The `.env` should have `SPROUT_REQUIRE_AUTH_TOKEN=false` for local dev.

---

## 4. Mint Test Credentials

### Option A: sprout-admin (full scopes including admin)

This mints a token with all CLI-relevant scopes (including `admin:channels`)
via direct DB access. Use this for testing admin operations (archive,
delete-channel, add/remove-channel-member).

```bash
DATABASE_URL=postgres://sprout:sprout_dev@localhost:5432/sprout \
cargo run -p sprout-admin -- mint-token \
  --name "cli-test" \
  --scopes "messages:read,messages:write,channels:read,channels:write,users:read,users:write,files:read,files:write,admin:channels"
```

This generates a keypair and prints:
- **Private key (nsec)** — save for `SPROUT_PRIVATE_KEY` testing

Export:

```bash
export SPROUT_RELAY_URL="http://localhost:3000"
export SPROUT_PRIVATE_KEY="nsec1..."   # from the mint output
```

### Scope reference

| Scope | Self-mintable | Needed for |
|-------|:---:|------------|
| `messages:read` | ✅ | `messages get`, `messages thread`, `messages search`, `feed get` |
| `messages:write` | ✅ | `messages send`, `messages edit`, `messages delete`, `reactions`, `messages vote` |
| `channels:read` | ✅ | `channels list`, `channels get`, `channels members` |
| `channels:write` | ✅ | `channels create`, `channels update`, `channels join`, `channels leave`, `channels topic`, `channels purpose` |
| `users:read` | ✅ | `users get`, `users presence` |
| `users:write` | ✅ | `users set-profile`, `users set-presence` |
| `files:read` | ✅ | — |
| `files:write` | ✅ | — |
| `admin:channels` | ❌ | `channels archive`, `channels unarchive`, `channels delete`, `channels add-member`, `channels remove-member` |

---

## 5. Unit Tests

```bash
cargo test -p sprout-cli
# Expected: see cargo test -p sprout-cli for current count

cargo clippy -p sprout-cli -- -D warnings
# Expected: zero warnings
```

---

## 6. Live Testing — Command by Command

Run each command, verify exit code 0 and check output. Most commands
return JSON (pipe through `jq .` to validate). Commands are ordered so
earlier ones create resources that later ones need.

### 6.1 Channels

```bash
# channels create (stream)
sprout channels create --name "test-stream" --type stream --visibility open \
  --description "CLI test channel" | jq .
# Save the channel ID:
CHANNEL_ID=$(sprout channels create --name "test-cli" --type stream --visibility open | jq -r '.id')

# channels create (forum) — needed for messages vote later
FORUM_ID=$(sprout channels create --name "test-forum" --type forum --visibility open | jq -r '.id')

# channels list
sprout channels list | jq .
sprout channels list --visibility open | jq .
sprout channels list --member | jq .

# channels get
sprout channels get --channel "$CHANNEL_ID" | jq .

# channels update
sprout channels update --channel "$CHANNEL_ID" --name "test-cli-updated" \
  --description "Updated" | jq .

# channels topic
sprout channels topic --channel "$CHANNEL_ID" --topic "Test topic" | jq .

# channels purpose
sprout channels purpose --channel "$CHANNEL_ID" --purpose "Testing" | jq .

# channels join (may already be a member from create)
sprout channels join --channel "$CHANNEL_ID" | jq .

# channels leave
sprout channels leave --channel "$CHANNEL_ID" | jq .

# Re-join so we can send messages
sprout channels join --channel "$CHANNEL_ID" | jq .

# channels archive (requires admin:channels scope)
sprout channels archive --channel "$CHANNEL_ID" | jq .

# channels unarchive
sprout channels unarchive --channel "$CHANNEL_ID" | jq .
```

### 6.2 Canvas

```bash
# canvas set
sprout canvas set --channel "$CHANNEL_ID" --content "# Test Canvas" | jq .

# canvas set from stdin
echo "# Canvas from stdin" | sprout canvas set --channel "$CHANNEL_ID" --content - | jq .

# canvas get
sprout canvas get --channel "$CHANNEL_ID" | jq .
```

### 6.3 Messages

```bash
# messages send
MSG=$(sprout messages send --channel "$CHANNEL_ID" --content "Hello from CLI test" | jq .)
echo "$MSG"
EVENT_ID=$(echo "$MSG" | jq -r '.id // .event_id')

# messages send with reply + broadcast
REPLY=$(sprout messages send --channel "$CHANNEL_ID" --content "Reply" \
  --reply-to "$EVENT_ID" --broadcast | jq .)
echo "$REPLY"
REPLY_ID=$(echo "$REPLY" | jq -r '.id // .event_id')

# messages send with mentions
sprout messages send --channel "$CHANNEL_ID" --content "Hey @someone" \
  --mention "0000000000000000000000000000000000000000000000000000000000000001" | jq .

# messages send from stdin — safe path for content with shell metacharacters
# (backticks, $vars, code blocks) that would otherwise be expanded by the shell.
echo 'Body with `backticks` and $vars stays literal.' \
  | sprout messages send --channel "$CHANNEL_ID" --content - | jq .

# messages get
sprout messages get --channel "$CHANNEL_ID" | jq .
sprout messages get --channel "$CHANNEL_ID" --limit 5 | jq .

# messages thread
sprout messages thread --channel "$CHANNEL_ID" --event "$EVENT_ID" | jq .

# messages search
sprout messages search --query "Hello" | jq .
sprout messages search --query "CLI test" --limit 5 | jq .

# messages edit
sprout messages edit --event "$EVENT_ID" --content "Edited by CLI test" | jq .

# messages delete
sprout messages delete --event "$REPLY_ID" | jq .
```

### 6.4 Diff Messages

```bash
# messages send-diff from stdin
echo '--- a/foo.rs
+++ b/foo.rs
@@ -1,3 +1,3 @@
-fn old() {}
+fn new() {}' | sprout messages send-diff \
  --channel "$CHANNEL_ID" \
  --diff - \
  --repo "https://github.com/example/repo" \
  --commit "abcdef1234567890abcdef1234567890abcdef12" | jq .

# messages send-diff with metadata
echo "diff content" | sprout messages send-diff \
  --channel "$CHANNEL_ID" \
  --diff - \
  --repo "https://github.com/example/repo" \
  --commit "abcdef1234567890abcdef1234567890abcdef12" \
  --file "src/main.rs" \
  --lang "rust" \
  --description "Refactored main" | jq .

# messages send-diff with branch + PR metadata
echo "diff content" | sprout messages send-diff \
  --channel "$CHANNEL_ID" \
  --diff - \
  --repo "https://github.com/example/repo" \
  --commit "abcdef1234567890abcdef1234567890abcdef12" \
  --parent-commit "1234567890abcdef1234567890abcdef12345678" \
  --source-branch "feature/cli" \
  --target-branch "main" \
  --pr 42 | jq .
```

### 6.5 Reactions

```bash
# Send a message to react to
REACT_MSG=$(sprout messages send --channel "$CHANNEL_ID" --content "React to this")
REACT_ID=$(echo "$REACT_MSG" | jq -r '.id // .event_id')

# reactions add
sprout reactions add --event "$REACT_ID" --emoji "👍" | jq .

# reactions get
sprout reactions get --event "$REACT_ID" | jq .

# reactions remove
sprout reactions remove --event "$REACT_ID" --emoji "👍" | jq .
```

### 6.6 DMs

```bash
# dms list
sprout dms list | jq .

# dms open (needs a real pubkey — use your own or a test one)
# Get your own pubkey first:
MY_PUBKEY=$(sprout users get | jq -r '.pubkey // .[0].pubkey // empty')
echo "My pubkey: $MY_PUBKEY"

# dms open with a synthetic pubkey (relay will create the user)
DM_RESULT=$(sprout dms open --pubkey "0000000000000000000000000000000000000000000000000000000000000001")
echo "$DM_RESULT" | jq .
DM_ID=$(echo "$DM_RESULT" | jq -r '.channel_id // .id // empty')

# dms add-member (requires messages:write scope — NOT admin:channels)
sprout dms add-member --channel "$DM_ID" \
  --pubkey "0000000000000000000000000000000000000000000000000000000000000002" | jq .
```

### 6.7 Users & Presence

```bash
# users get — own profile (0 pubkeys)
sprout users get | jq .

# users get — single pubkey
sprout users get --pubkey "$MY_PUBKEY" | jq .

# users get — batch (2+ pubkeys)
sprout users get --pubkey "$MY_PUBKEY" --pubkey "$MY_PUBKEY" | jq .

# users set-profile
sprout users set-profile --name "CLI Test Agent" --about "Testing sprout-cli" | jq .

# users presence
sprout users presence --pubkeys "$MY_PUBKEY" | jq .

# users set-presence
sprout users set-presence --status online | jq .
sprout users set-presence --status away | jq .
sprout users set-presence --status offline | jq .
```

### 6.8 Channel Members (add/remove require admin:channels)

```bash
# channels add-member
sprout channels add-member --channel "$CHANNEL_ID" \
  --pubkey "0000000000000000000000000000000000000000000000000000000000000001" \
  --role member | jq .

# channels members
sprout channels members --channel "$CHANNEL_ID" | jq .

# channels remove-member
sprout channels remove-member --channel "$CHANNEL_ID" \
  --pubkey "0000000000000000000000000000000000000000000000000000000000000001" | jq .
```

### 6.9 Workflows

```bash
# workflows create
# NOTE: trigger uses `on:` tag (serde internally tagged enum).
# Valid triggers: message_posted, reaction_added, diff_posted, schedule, webhook
# Steps use `action:` tag: send_message, send_dm, set_channel_topic, add_reaction, etc.
WF=$(sprout workflows create --channel "$CHANNEL_ID" \
  --yaml 'name: test-wf
trigger:
  on: webhook
steps:
  - id: step1
    action: send_message
    text: "Hello from workflow"' | jq .)
echo "$WF"
WF_ID=$(echo "$WF" | jq -r '.id')

# workflows list
sprout workflows list --channel "$CHANNEL_ID" | jq .

# workflows get
sprout workflows get --workflow "$WF_ID" | jq .

# workflows update
sprout workflows update --workflow "$WF_ID" \
  --yaml 'name: test-wf-updated
trigger:
  on: webhook
steps:
  - id: step1
    action: send_message
    text: "Updated"' | jq .

# workflows trigger
sprout workflows trigger --workflow "$WF_ID" | jq .

# workflows runs
sprout workflows runs --workflow "$WF_ID" | jq .

# workflows approve — requires a workflow run waiting for approval
# This is hard to test ad-hoc without a workflow that has an approval gate.
# Test the validation instead:
sprout workflows approve --token "00000000-0000-0000-0000-000000000000" 2>&1 || true
# Should fail with relay error (token not found), not a validation error
# To test the deny path: sprout workflows approve --token <UUID> --approved false

# workflows delete
sprout workflows delete --workflow "$WF_ID" | jq .
```

### 6.10 Feed

```bash
sprout feed get | jq .
sprout feed get --limit 5 | jq .
```

### 6.11 Forum & Voting

```bash
# Send a forum post (kind 45001) to the forum channel
FORUM_POST=$(sprout messages send --channel "$FORUM_ID" \
  --content "Forum post for vote testing" --kind 45001 | jq .)
echo "$FORUM_POST"
FORUM_EVENT_ID=$(echo "$FORUM_POST" | jq -r '.id // .event_id')

# messages vote (up)
sprout messages vote --event "$FORUM_EVENT_ID" --direction up | jq .

# messages vote (down)
sprout messages vote --event "$FORUM_EVENT_ID" --direction down | jq .
```

### 6.12 Notes (NIP-23 long-form, kind:30023)

Editable team-knowledge notes keyed by `(kind:30023, you, d=slug)`. `set` is an
idempotent upsert; `rm` is a NIP-09 a-tag deletion. Output is plain text (refs),
not JSON — except `get`/`ls`, which emit JSON.

```bash
# set (first publish — --title required, body from stdin)
cat <<'EOF' | sprout notes set --name dco-check --title "DCO Check" \
  --summary "How we verify DCO" --tag dco --tag ci --content -
Run `git log --format='%(trailers:key=Signed-off-by)'` ...
EOF
# → prints event_id / naddr / coordinate / slug / title

# set (edit — omit --title to carry it forward; published_at preserved)
echo "Updated body." | sprout notes set --name dco-check --content -

# get by name (own author resolves directly; cross-author #d query otherwise)
sprout notes get --name dco-check | jq .
sprout notes get --name dco-check --content-only

# get by naddr (exact coordinate; paste the naddr from a set/get above)
sprout notes get --naddr "$NADDR" | jq .

# ls (own by default; --author all across the team; --tag filters)
sprout notes ls | jq .
sprout notes ls --tag dco | jq .
sprout notes ls --author all --limit 10 | jq .

# rm (NIP-09 a-tag deletion; subsequent get must 404)
sprout notes rm --name dco-check
# → prints deleted <coordinate> / deletion <event-id>
sprout notes get --name dco-check   # exits non-zero: not found

# rm of a slug you never published → NotFound, no kind:5 emitted
sprout notes rm --name does-not-exist   # exits non-zero
```

---

## 7. Error Path Testing

Verify the CLI produces correct JSON on stderr and correct exit codes.

```bash
# Exit 1: Invalid UUID
sprout channels get --channel "not-a-uuid" 2>&1; echo "exit: $?"
# stderr: {"error":"user_error","message":"invalid UUID: not-a-uuid"}
# exit: 1

# Exit 1: Invalid hex64
sprout messages delete --event "not-hex" 2>&1; echo "exit: $?"
# stderr: {"error":"user_error","message":"must be a 64-character hex string: not-hex"}
# exit: 1

# Exit 1: Invalid --type value
sprout channels create --name x --type invalid --visibility open 2>&1; echo "exit: $?"
# stderr: {"error":"user_error","message":"--type must be 'stream' or 'forum' (got: invalid)"}
# exit: 1

# Exit 1: Invalid --direction value
sprout messages vote --event "$(printf '0%.0s' {1..64})" \
  --direction sideways 2>&1; echo "exit: $?"
# exit: 1

# Exit 1: Empty body guard
sprout users set-profile 2>&1; echo "exit: $?"
# exit: 1 (at least one field required)

# Exit 3: No auth configured
env -u SPROUT_PRIVATE_KEY \
  cargo run -p sprout-cli -- channels list 2>&1; echo "exit: $?"
# stderr: {"error":"auth_error","message":"SPROUT_PRIVATE_KEY is required (use --private-key or set env var)"}
# exit: 3

# Exit 2: Non-existent channel (valid UUID)
sprout channels get --channel "00000000-0000-0000-0000-000000000000" 2>&1; echo "exit: $?"
# stderr: {"error":"relay_error","message":"..."}
# exit: 2
```

---

## 8. Auth Testing

Test authentication.

```bash
# Private key (SPROUT_PRIVATE_KEY)
SPROUT_PRIVATE_KEY="nsec1..." sprout channels list | jq .
# Should succeed

# No auth → exit 3
env -u SPROUT_PRIVATE_KEY \
  cargo run -p sprout-cli -- channels list 2>&1; echo "exit: $?"
# stderr: {"error":"auth_error","message":"SPROUT_PRIVATE_KEY is required (use --private-key or set env var)"}
# exit: 3
```

---

## 9. Cleanup

```bash
# Delete test channels
sprout channels delete --channel "$CHANNEL_ID" | jq .
sprout channels delete --channel "$FORUM_ID" | jq .
```

---

## 10. Checklist

| # | Command | Tested | Notes |
|---|---------|:------:|-------|
| 1 | `messages send` | ☐ | Basic, reply, broadcast, mentions, stdin |
| 2 | `messages send-diff` | ☐ | Stdin, metadata, branch/PR |
| 3 | `messages edit` | ☐ | |
| 4 | `messages delete` | ☐ | |
| 5 | `messages get` | ☐ | With limit |
| 6 | `messages thread` | ☐ | |
| 7 | `messages search` | ☐ | With limit |
| 8 | `messages vote` | ☐ | Up and down |
| 9 | `channels list` | ☐ | With visibility, member |
| 10 | `channels get` | ☐ | |
| 11 | `channels create` | ☐ | Stream and forum |
| 12 | `channels update` | ☐ | |
| 13 | `channels topic` | ☐ | |
| 14 | `channels purpose` | ☐ | |
| 15 | `channels join` | ☐ | |
| 16 | `channels leave` | ☐ | |
| 17 | `channels archive` | ☐ | Needs admin:channels |
| 18 | `channels unarchive` | ☐ | Needs admin:channels |
| 19 | `channels delete` | ☐ | Needs admin:channels |
| 20 | `channels members` | ☐ | |
| 21 | `channels add-member` | ☐ | Needs admin:channels |
| 22 | `channels remove-member` | ☐ | Needs admin:channels |
| 23 | `canvas get` | ☐ | |
| 24 | `canvas set` | ☐ | Direct and stdin |
| 25 | `reactions add` | ☐ | |
| 26 | `reactions remove` | ☐ | |
| 27 | `reactions get` | ☐ | |
| 28 | `dms list` | ☐ | |
| 29 | `dms open` | ☐ | |
| 30 | `dms add-member` | ☐ | Needs messages:write |
| 31 | `users get` | ☐ | Self, single, batch |
| 32 | `users set-profile` | ☐ | |
| 33 | `users presence` | ☐ | |
| 34 | `users set-presence` | ☐ | online, away, offline |
| 35 | `workflows list` | ☐ | |
| 36 | `workflows create` | ☐ | |
| 37 | `workflows update` | ☐ | |
| 38 | `workflows delete` | ☐ | |
| 39 | `workflows trigger` | ☐ | |
| 40 | `workflows runs` | ☐ | |
| 41 | `workflows get` | ☐ | |
| 42 | `workflows approve` | ☐ | Validation only (needs approval gate); bare = approve, `--approved false` = deny |
| 43 | `feed get` | ☐ | |
| 44 | `social publish` | ☐ | |
| 45 | `social set-contacts` | ☐ | |
| 46 | `social event` | ☐ | |
| 47 | `social notes` | ☐ | |
| 48 | `social contacts` | ☐ | |
| 49 | `repos create` | ☐ | |
| 50 | `repos get` | ☐ | |
| 51 | `repos list` | ☐ | |
| 52 | `upload file` | ☐ | |
| 53 | `pack validate` | ☐ | Local, no relay |
| 54 | `pack inspect` | ☐ | Local, no relay |
| 55 | `notes set` | ☐ | First publish, edit/carry, --clear-tags, ambiguity, empty-stdin guard |
| 56 | `notes get` | ☐ | By name, by naddr, --content-only, cross-author, ambiguous → exit 1 |
| 57 | `notes ls` | ☐ | Own, --author all, --tag, --limit |
| 58 | `notes rm` | ☐ | Delete→get 404, double-delete idempotent, missing slug → NotFound |
