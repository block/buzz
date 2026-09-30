# Hosted community bots → NIP-AM 44200 (token chips)

Captain / Mo / Stitch / Korg / Quasar (and peers) on **buzz.huladesk.com** run as
**VPS last-miles**: one `buzz-acp@<id>` systemd unit per OpenClaw agent, stdio-ACP
into OpenClaw (`BUZZ_ACP_AGENT_COMMAND=node` + `…/openclaw/…/index.js`), relay
`wss://buzz.huladesk.com`. They already publish kind **24200** (turn transcripts)
with `#p` = interacting owner. Token chips in desktop need kind **44200**
(`agent_turn_metric`) from the same harness.

## Why chips were empty

OpenClaw emits standard ACP `usage_update` with **`used` / `size`** (session
token proxy + context window), not Claude `cost.amount` and not goose
input/output. Older buzz-acp only mapped Claude cost → `take_turn_usage()` often
returned `None` → `publish_agent_turn_metric` no-op → **0 × 44200** in archives.

## Fix (this PR)

In `buzz-acp`:

1. Detect OpenClaw spawn even when command basename is `node` (sniff args for
   `openclaw`) → `StandardAdapterKind::OpenClaw`.
2. On `usage_update`, record session-cumulative `used`; delta →
   `TurnUsage.turn_total_tokens` / `cumulative_total_tokens`.
3. Existing `publish_agent_turn_metric` path encrypts 44200 to the registered
   owner (`#p`), same as local buzz-acp.

Desktop chrome (PR #177) already renders token chips from archived 44200 when
“Show agent thinking” is on. No desktop change required for hosted bots once
VPS `buzz-acp` is rebuilt.

## Deploy on dohula (prod last-miles)

Hosted bots are **not** Helm/compose in this repo — they are user systemd units
on the OpenClaw VPS (commonly called **dohula**).

1. **Build** buzz-acp from main (or this PR branch) on a Mac/Linux builder with
   the same target as the VPS (usually `x86_64-unknown-linux-gnu`):
   ```bash
   cargo build -p buzz-acp --release
   # binary: target/release/buzz-acp  (or cross-compile if building on macOS)
   ```
2. **Copy** the binary to the VPS (example):
   ```bash
   scp target/release/buzz-acp dohula:~/.local/bin/buzz-acp.new
   ssh dohula 'install -m 755 ~/.local/bin/buzz-acp.new ~/.local/bin/buzz-acp'
   ```
3. **Restart** each community last-mile unit (names may vary; confirm with
   `systemctl --user list-units 'buzz-acp@*'`):
   ```bash
   ssh dohula 'systemctl --user restart buzz-acp@captain buzz-acp@mo buzz-acp@korg buzz-acp@quasar'
   # add stitch / main / others when those units exist
   ```
4. **Verify**
   - `journalctl --user -u buzz-acp@captain -n 80` — clean start, no spawn panic.
   - Trigger a Captain turn from desktop; archive should gain a new **44200**
     with `#p` = JM (owner) and `totalTokens` set.
   - Desktop token chip under the reply lights up after relay sync (thinking
     chrome enabled).

Optional: units that still lack `reasoning_level=stream` also need this binary
for Thought chips (`apply_startup_reasoning_stream` from #177). Same restart.

## Local parity

Desktop-spawned OpenClaw / managed agents pick up the same buzz-acp sidecar on
the next desktop rebuild/bundle. Hosted bots only move when the VPS binary is
replaced.

## Related

- NIP-AM: `docs/nips/NIP-AM.md` (44200 / `#p` owner)
- Last-mile publish: `crates/buzz-acp/src/last_mile.rs`, README “VPS last-mile”
- Per-message chrome: PR #177 (`77979c8a6`)
