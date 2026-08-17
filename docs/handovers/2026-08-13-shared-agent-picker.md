# Shared agent picker fix

## Verified

- Reproduced the live failure from Andrea's event: typed `@codex` had no `p` tag because Desktop's relay directory read kind `10100`, which managed agents do not publish.
- `list_relay_agents` now joins kind `30177` access policy with the agent's NIP-OA-verified kind `0` owner and relay-signed kind `39002` channel membership.
- The directory preserves self-authored kind `10100` headless agents, while verified kind `30177` policy wins on collisions for Desktop-managed agents.
- Directory and membership reads use composite-cursor pagination, profile reads are bounded in 250-author chunks, and optional enrichment failures degrade to the remaining verified source.
- Forged policy, forged membership, malformed agent IDs, missing owner proof, and revoked owner proof fail closed.
- Equal-timestamp replaceable heads resolve deterministically by event ID.
- `just desktop-tauri-test`: full Rust workspace suite green after the hardening pass.
- `just desktop-tauri-fmt-check` and `just desktop-tauri-clippy`: green.

## Pending

- Open the upstream PR and wait for a Buzz Desktop release.
- Verify Andrea can select Codex, Opus, and Fable after installing that release.

## Related work

- Supersedes the incomplete tradeoffs in #4713, #4714, #4716, #5483, #5546, and #5691 by preserving both deployed directory shapes without trusting owner-claimed agent identities or agent-claimed channel membership.
