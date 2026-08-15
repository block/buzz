# Native private remote pilot acceptance

Date: 9 August 2026  
Endpoint: `https://matthews-macbook-pro-1.tailf29f2c.ts.net/`  
Rollback: `TAILSCALE_BE_CLI=1 /Applications/Tailscale.app/Contents/MacOS/Tailscale serve reset`

This is the acceptance runbook for using the native Command Adviser iPhone
client with the MacBook-hosted relay. The endpoint is available inside the
user's tailnet only. Tailscale Funnel and public ingress must remain disabled.

## Automated and MacBook preflight

1. Confirm the relay is available at `http://127.0.0.1:3000` and returns its
   NIP-11 document when requested with `Accept: application/nostr+json`.
2. Confirm the bundled Tailscale client is connected and accepts tailnet DNS:

   ```bash
   TAILSCALE_BE_CLI=1 /Applications/Tailscale.app/Contents/MacOS/Tailscale set --accept-dns=true
   ```

3. Configure the tailnet-only TLS proxy:

   ```bash
   TAILSCALE_BE_CLI=1 /Applications/Tailscale.app/Contents/MacOS/Tailscale serve --bg --yes http://127.0.0.1:3000
   ```

4. Verify the status says `tailnet only`, the proxy target is literal loopback,
   and the HTTPS endpoint returns the same authenticated Buzz NIP-11 document:

   ```bash
   TAILSCALE_BE_CLI=1 /Applications/Tailscale.app/Contents/MacOS/Tailscale serve status
   TAILSCALE_BE_CLI=1 /Applications/Tailscale.app/Contents/MacOS/Tailscale funnel status
   curl --fail --header 'Accept: application/nostr+json' \
     https://matthews-macbook-pro-1.tailf29f2c.ts.net/
   ```

Both status commands must label the endpoint `tailnet only`. No Funnel-enabled
listener is acceptable.

## Verified candidate checkpoint

The 9 August 2026 automated checkpoint passed:

- full repository `just ci` gate;
- Dart formatting, Flutter analysis, and all 1,267 Flutter tests;
- native `RunnerTests` on an iPhone 17 Pro simulator through `xcodebuild test`;
- signed macOS app and bundled sidecar verification; and
- a fresh tailnet HTTPS request returning the live Buzz NIP-11 document.

The installable Mac candidate is:

```text
desktop/src-tauri/target/aarch64-apple-darwin/release/bundle/dmg/Command Adviser_0.5.8_aarch64.dmg
SHA-256 394a1fc9ad6902412d931c1edaace23340367b4e68a51d73247ffd0a7de0d368
```

This locally signed candidate is intentionally not notarized. The remaining
acceptance boundary is the real iPhone journey below; simulator and unit tests
do not prove mobile-data reachability or the complete QR/SAS exchange.

## Physical iPhone gate

These are the only steps that require the user.

1. Install or open the native iPhone candidate and connect Tailscale to the same
   tailnet as the MacBook.
2. In Command Adviser on the Mac, open **Settings > Mobile**. Enter
   `https://matthews-macbook-pro-1.tailf29f2c.ts.net/` under **Private iPhone
   relay**, then select **Start pairing**.
3. Scan the QR code with the iPhone app. Compare the six-digit SAS on both
   devices and accept only an exact match.
4. On the iPhone, turn off Wi-Fi so the phone is on mobile data, but leave
   Tailscale connected. Confirm an existing channel or DM loads historical
   messages.
5. Send a uniquely worded DM from the iPhone and confirm it appears once on the
   Mac. Reply from the Mac and confirm it appears once on the iPhone.
6. Message one Command Adviser managed agent from the iPhone. Confirm the
   request reaches the MacBook-managed agent and its signed response returns to
   the iPhone; models, RAG, Memory, and credentials remain on the MacBook.
7. Disable Tailscale on the iPhone. Confirm the app shows **Private relay
   unavailable** and **Check Tailscale or VPN**, cannot send through a hosted
   fallback, and does not remove the community.
8. Re-enable Tailscale. Confirm history and normal messaging resume without a
   duplicate of either acceptance message.
9. Switch to any existing community and back. Confirm its stored relay and
   identity remain unchanged.

Record the candidate version, time, iPhone network state, message event IDs,
agent/thread used, and pass/fail result. Do not mark remote access accepted
until every physical step passes.

## Failure interpretation

- **Private relay unavailable — Check Tailscale or VPN:** restore Tailscale on
  both devices and confirm the MacBook is awake and Command Adviser is running.
- **Authentication failed:** generate a new QR in Command Adviser and re-pair;
  do not delete another community to work around it.
- A certificate, hostname, or public-domain substitution must be rejected. The
  desktop accepts only a root HTTPS `*.ts.net` origin for private pairing.

## Rollback

Disable the Serve listener without changing Buzz data, identities, or the local
relay:

```bash
TAILSCALE_BE_CLI=1 /Applications/Tailscale.app/Contents/MacOS/Tailscale serve reset
```

If tailnet DNS also needs to return to its pre-pilot state on this Mac, run:

```bash
TAILSCALE_BE_CLI=1 /Applications/Tailscale.app/Contents/MacOS/Tailscale set --accept-dns=false
```

Clear **Private iPhone relay** in Desktop Settings to restore ordinary pairing
against the current workspace relay. Existing signed Buzz data is not migrated
or deleted by either rollback.
