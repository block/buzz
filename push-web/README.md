# Hula Buzz Web Push sender (not deployed)

Delivers Web Push (VAPID) to the Hula Buzz PWA when the app is closed: @mentions, DMs, thread replies to your
messages, and approvals (kinds 46010-46012). Honors each user's notification toggles (synced from the web app).

- Never holds or asks for user private keys. Clients register with a NIP-98 signature from their own signer.
- Push payloads carry no message content: only sender name, channel name, a category line and a routing target.
- Subscriptions live in `DATA_DIR/subscriptions.json` (0600). Endpoints are restricted to real browser push
  services (Apple, Google, Mozilla, Microsoft) to prevent SSRF.
- Reads the relay with its OWN service nostr identity (NIP-42 AUTH). It only sees what that identity may read.

## Run locally
    npm install && npm test
    VAPID_PUBLIC_KEY=... VAPID_PRIVATE_KEY_FILE=./vapid-private.key RELAY_URL=wss://buzz.huladesk.com \
    SERVICE_SECRET_KEY_FILE=./service.key PUBLIC_URL=https://push.hulabuzz.huladesk.com npm start

## Enable in the web client
Build the web app with `VITE_BUZZ_WEB_PUSH_URL=https://<push host>`. Without it every push surface stays hidden.
