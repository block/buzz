# Shared agent picker fix

## Verified

- Reproduced the live failure from Andrea's event: typed `@codex` had no `p` tag because Desktop's relay directory read kind `10100`, which managed agents do not publish.
- `list_relay_agents` now joins kind `30177` access policy with the agent's NIP-OA-verified kind `0` owner and relay-signed kind `39002` channel membership.
- Forged policy, forged membership, malformed agent IDs, missing owner proof, and revoked owner proof fail closed.
- `just desktop-tauri-test`: 2,435 Rust tests, 0 failures, 15 ignored.
- `just desktop-tauri-fmt-check` and `just desktop-tauri-clippy`: green.

## Pending

- Open the upstream PR and wait for a Buzz Desktop release.
- Verify Andrea can select Codex, Opus, and Fable after installing that release.

## Known limit

- Relay queries still inherit the existing 1,000-event response cap. Large-community pagination is separate follow-up work.
