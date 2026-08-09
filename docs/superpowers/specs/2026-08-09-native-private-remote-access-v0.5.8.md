# Native private remote access — v0.5.8 compatibility specification

Date: 9 August 2026
Status: frozen for a later basic pilot after Keeper MVP

## Outcome

The native Buzz iPhone client will reach the MacBook-hosted Command Adviser over
the user's private Tailscale/VPN network. The MacBook remains authoritative for
the relay, managed agents, models, RAG, Memory, signed events, and application
data. The phone is a native remote client, not a second Command Adviser stack.

The v0.5.8 base already provides native mobile pairing, identity storage,
multiple-community relay configuration, authenticated Nostr messaging, history,
media, and connection-state handling. Tailscale supplies transport reachability.
The pilot therefore needs configuration and focused acceptance work, not a new
gateway or synchronization protocol.

## Basic pilot journey

1. Tailscale is installed and authenticated on the MacBook and iPhone.
2. The MacBook exposes the existing Buzz relay only on an address reachable
   inside the tailnet. Public ingress and Tailscale Funnel remain disabled.
3. The user pairs the native iPhone client using the existing QR/SAS flow and
   stores the MacBook relay as a community.
4. From mobile data outside the home LAN, the phone loads channel/DM history,
   sends and receives a message, and completes one Command Adviser exchange.
5. Disabling Tailscale makes the client visibly offline; it does not silently
   switch to a hosted relay or public endpoint.

## Reused v0.5.8 components

| Need | Existing component | Pilot use |
| --- | --- | --- |
| Device bootstrap | NIP-AB QR/SAS mobile pairing | Pair the existing identity/community without new credentials |
| Relay configuration | mobile community and relay providers | Store the private tailnet relay URL as an ordinary community endpoint |
| Messaging and history | native mobile signed-event relay client | Use existing authenticated send, receive, subscriptions, and history |
| Agent interaction | existing Command Team DMs and relay-managed agents | Run agents on the MacBook; return signed messages to the phone |
| Failure visibility | mobile relay lifecycle and closed-state policy | Show VPN/relay loss as offline with a retry path |
| Private transport | Tailscale/VPN | Route traffic only; do not add application data semantics |

## Network and data boundary

- The configured relay URL uses the MacBook's stable tailnet name or address and
  the existing relay WebSocket port/path.
- No hosted relay, reverse proxy on the public internet, Telegram bridge,
  Tailscale Funnel, or cloud copy of Command Adviser data is introduced.
- Existing relay authentication and Nostr signatures remain mandatory even
  inside the VPN.
- Model routing remains unchanged. Cloud-primary or local-fallback execution
  still occurs on the MacBook according to Command Adviser settings.
- RAG, Memory, Apple data, and model credentials are never copied to the phone;
  the phone receives only ordinary Buzz conversation results and media it is
  authorized to access.

## Pilot implementation slices

1. Document and validate a private relay bind/advertise configuration that is
   reachable on the tailnet without public ingress.
2. Add a focused mobile validation path for a private `ws://` or `wss://`
   tailnet endpoint, preserving existing URL safety rules.
3. Add clear connection-state copy for VPN-unreachable versus authentication or
   relay errors where current mobile diagnostics are insufficient.
4. Add a repeatable end-to-end acceptance runbook using the existing pairing
   and message paths.

## Acceptance tests

- A paired iPhone on mobile data can load history and send/receive a signed DM
  through the private tailnet relay.
- The iPhone completes one real Command Adviser turn while all model and source
  work remains on the MacBook.
- Turning off Tailscale prevents connection, shows a visible offline/error
  state, and does not fall back to any public or hosted relay.
- An unpaired identity and an invalid relay certificate/host cannot access the
  community.
- Reconnecting Tailscale resumes the ordinary session without duplicate sent
  messages or loss of already acknowledged messages.
- Existing desktop and mobile communities continue to work after adding the
  private Command Adviser community.

## Deferred hardening

Only after the basic pilot proves useful will the project assess:

- APNs and notification-triggered navigation;
- background wake and long-suspension reconnection;
- an encrypted durable mobile outbox for messages composed while disconnected;
- additional device-attestation or revocation UX; and
- always-on relay hosting away from the MacBook.

These are later resilience features, not prerequisites for the first private
remote journey.
