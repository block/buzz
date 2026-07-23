# Lightning Wallet — Phase 1 (Nostr Wallet Connect)

`draft`

## Abstract

This document specifies **Phase 1** of Lightning payments in Buzz: every user —
and, in a bounded receive-only form, every agent — can link an **external**
Lightning wallet over [NIP-47 Nostr Wallet Connect (NWC)](https://github.com/nostr-protocol/nips/blob/master/47.md),
show a receiving address, and send and receive sats. Buzz **never custodies
funds**: the money plane runs client-side between the user's own wallet service
and the wallet's relay, and never touches the Buzz relay or any Buzz-operated
server.

The design is driven by two constraints that pull against each other and, in
resolving them, produce the shape of the system:

- **Eliminate special cases.** There is no "human path" and "agent path"; there
  is no "bolt11 vs lightning-address" branch in the pay path; there is no
  "wallet capability" speculation. An agent is a keypair that posts events,
  exactly like a human. A payment always resolves to a bolt11 before it is paid.
- **Dependencies point inward, and every boundary must earn its keep.** The
  payment domain (types + verification) is pure and lives in `buzz-core` with
  zero network dependency. Exactly **one** meaningful interface — `WalletService`
  — separates use-cases from the wallet transport, justified by two real
  implementations on day one (`NwcWalletService` and `FakeWalletService`) and by
  being the exact seam a future custodial/LDK adapter would slot into. No
  ceremony layers beyond that.

## Scope

**In scope (Phase 1):**

- Link an external wallet via an NWC connection string (paste or QR).
- Show balance (when the wallet exposes it), a receive address / QR, send, and
  receive.
- Send to any Lightning Address (LUD-16) and to any other Buzz user who
  publishes one.
- In-chat **payment requests** and decorative **receipts**, scoped to a channel
  or DM.
- Agents that talk to humans can **request** payment (receive) and prove they
  were paid — verified against **their own wallet**, not a posted event.

**Out of scope (deferred):**

- **Autonomous agent spending** — requires extending the NIP-OA `conditions`
  grammar with budget clauses and a relay spend scope. See *Phase 2*.
- **Custodial wallet** operated by the relay. Reachable later as a single
  `WalletService` adapter without redesign (see [§7](#7-why-nwc-is-a-detail-screaming-payments)).
- **NIP-57 zaps** as a first-class "monetary reaction". Buildable on top of the
  payment-request primitive; not required for Phase 1.

**Non-negotiable invariant:** the NWC secret (which can spend, up to the
wallet's own limits) lives **only** in native secure storage and the native
backend. The web/Flutter UI never holds it. **Every send requires explicit human
confirmation.**

---

## 1. Two-plane architecture

The insight that keeps custody and liability at zero: **the movement of money
never passes through the Buzz relay or any Buzz server.**

```
┌──────────────────── TRANSPORT PLANE (NWC / off-Buzz) ─────────────────────┐
│                                                                           │
│  Buzz client (native backend) ◄──── wallet's own relay ────► user wallet  │
│     │  NWC secret in secure storage    (e.g. relay.getalby.com)           │
│     │  kind 23194/23195/23196, NIP-44 encrypted                           │
└─────┼─────────────────────────────────────────────────────────────────────┘
      │
┌─────┼──────────────────── DISCOVERY + UX PLANE (on Buzz) ──────────────────┐
│     ▼                                                                       │
│  Buzz relay (NIP-29)                                                        │
│  • kind:0 `lud16`         → public receiving address, already synced        │
│  • KIND_PAYMENT_REQUEST   → "Pay N sats" card in channel/DM (h-scoped)      │
│  • KIND_PAYMENT_RECEIPT   → decorative "Paid ✓" in the thread               │
└────────────────────────────────────────────────────────────────────────────┘
```

- **Transport plane (NWC):** a *second* WebSocket, to the relay the user's
  wallet chose (named in the connection string). Owned entirely by the native
  backend. The Buzz relay is not involved.
- **Discovery / UX plane (Buzz relay):** only what *others* need in order to pay
  you (your `lud16`) and the in-chat interactions. Reuses the existing NIP-29
  pipeline: `h`-scoping, fan-out, auth. The relay stays a dumb transport for
  these events — **no payment logic in the relay**.

---

## 2. Clean layering (dependencies point inward)

```
Entities        buzz-core        Amount(msat), Invoice, PaymentRequest, Receipt,
  pure, no I/O                    verify(preimage → payment_hash), request validation
      ▲
Use cases       buzz-wallet      link_wallet, send_payment, request_payment,
  ports only                     resolve_address, check_incoming
      ▲
Ports (traits)                   WalletService { make_invoice, pay_invoice,
                                   get_balance?, lookup }   +   LnurlResolver
      ▲
Adapters                         NwcWalletService (rust-nostr `nwc`)   FakeWalletService
                                 HttpLnurlResolver                     (tests, offline)
      ▲
Frameworks/drivers               Tauri commands · Flutter UI · buzz-cli · relay ingest
```

Mapped onto existing Buzz conventions:

| Layer | Crate / location | Responsibility |
|---|---|---|
| Entities | `buzz-core` | Pure payment types + `verify(preimage, payment_hash)` + request validation. **Zero network deps.** Fits "core types, event verification". |
| Event encoding | `buzz-sdk` | Typed builders for the two new kinds. Fits "typed Nostr event builders". |
| Use-cases + ports + adapters | **`buzz-wallet`** (new) | `WalletService` / `LnurlResolver` traits; `NwcWalletService`, `HttpLnurlResolver`, `FakeWalletService`. Consumed by the Tauri backend and `buzz-cli`. |
| Transport (on Buzz) | `buzz-relay` | Thin ingest + fan-out of the two kinds, `h`-scoped. **No payment logic.** |
| Drivers | desktop / mobile / CLI | Call use-cases; hold the secret only in the native backend. |

**SOLID notes.** SRP: LNURL resolution (`LnurlResolver`) is separated from
paying (`WalletService.pay_invoice`) — the pay path never speaks HTTP. DIP:
use-cases depend on the `WalletService` trait, not on rust-nostr `nwc`. ISP: the
port is small and capabilities are queried (`get_balance` is optional; many NWC
wallets do not expose it) rather than a god-interface. OCP: zaps / custodial /
LDK arrive as new adapters or use-cases, not edits to existing ones.

**Which boundaries earn their keep** (and which we refuse):

| Boundary | Pays for itself? | Verdict |
|---|---|---|
| `WalletService` port | Yes — `FakeWalletService` (test) + `NwcWalletService` (prod), and it is the exact seam for a future custodial adapter | Keep |
| `LnurlResolver` port | Yes — `HttpLnurlResolver` + a fake in tests | Keep |
| Pure payment domain in `buzz-core` | Yes — makes the trust logic verifiable offline | Keep |
| A DTO "interface-adapter" layer between use-cases and event encoding | No — `buzz-sdk` already builds events | Reject |
| An interface around every type | No — plain data, pure functions | Reject |

Net: **exactly one meaningful interface** (plus the resolver); plain data and
pure functions everywhere else.

---

## 3. Data model

### 3.1 Event kinds (`buzz-core/src/kind.rs`)

Two new kinds — deliberately distinct so consumers dispatch on the kind number
(no stringly-typed `type` tag branching). Follow the registry process: add the
`pub const`, append to `ALL_KINDS`, place into the right gating slices, add the
compile-time range `assert!`, then wire ingest in `buzz-relay`.

| Kind | Band | Type | Purpose |
|---|---|---|---|
| `lud16` on **kind:0** (reuse) | standard | replaceable | **Receiving address.** The primary, interoperable receive primitive. |
| `KIND_PAYMENT_REQUEST` | 40000s messaging | regular, `h`-scoped | "Pay N sats" card. Tags: `amount` (msat), `memo`, `bolt11` **or** `lud16`, `h` (channel), `p` (payee), `expiry`. |
| `KIND_PAYMENT_RECEIPT` | 40000s messaging | regular, `h`-scoped | References the request via `e`. Carries `payment_hash`, `preimage`, `amount`. Renders "Paid ✓". |

Deliberately **not** added: a `KIND_WALLET_CAPABILITY` advertisement — it was
optional and speculative, so it does not exist. `lud16` in kind:0 is sufficient.

A `bolt11` posted in a **public channel** is payable by whoever pays first. For
1:1 payments, use `lud16` (a fresh invoice per payer) or an encrypted DM.

### 3.2 Trust is local (the receipt is decorative)

`KIND_PAYMENT_RECEIPT` is **not** on the trust path. Anyone can post a fake
receipt. The source of truth is **your own wallet**: the payee learns of the
payment from their wallet's `payment_received` notification (kind 23196) or
`lookup_invoice`; the payer holds the `preimage` returned by their own
`pay_invoice`. The receipt event only makes "Paid ✓" visible to third parties in
the thread. An agent confirms it was paid by querying its own wallet — never by
trusting a posted preimage.

### 3.3 Relay storage (`buzz-db`)

- Add a nullable `lud16` column to `users` (migration `00NN_*.sql`, mirrored in
  `schema/schema.sql`), populated during kind:0 ingest, so any user's receiving
  address is available without refetching the profile.
- No other tables in Phase 1: requests/receipts are events; balances live in the
  user's wallet and are never persisted on the relay.

### 3.4 NWC secret storage (client)

| Client | Location | Notes |
|---|---|---|
| Desktop | `desktop/src-tauri/src/secret_store.rs` (OS keyring) | **Not** `identity.key`; **not** on any env-read path. Keyed per community. |
| Mobile | `flutter_secure_storage`, per community | Mirrors the `nsec` in `community_storage.dart`. |
| CLI | env `BUZZ_NWC_URI` or a `0600` config file | Mirrors `BUZZ_PRIVATE_KEY`. |
| Agent (receive-only) | injected as `BUZZ_NWC_URI` (receive-only) via `buzz-acp` | See [§6](#6-agents-that-interact-with-humans). |

**Community switching.** The NWC secret and the live NWC connection are
community-scoped. The connection is a module-level singleton, so a
`resetWalletState()` must be wired into `resetCommunityState()` in
`desktop/src/features/communities/useCommunityInit.ts`, alongside a reset of the
balance/history cache — otherwise the old community's wallet leaks into the new
one.

---

## 4. `buzz-wallet` port surface

```
link(uri)                                   -> WalletHandle   // validate, read get_info + capabilities (13194)
get_balance()                               -> Option<msat>   // capability-gated
make_invoice(amount, memo)                  -> bolt11          // "receive"
pay_invoice(bolt11)                         -> preimage        // "send"
lookup_invoice(payment_hash)                -> InvoiceStatus
list_transactions()                         -> [Tx]
```

`LnurlResolver.resolve(lud16, amount, memo) -> bolt11` lives *beside* the wallet,
not inside it: a lightning address is just a way to *produce* a bolt11. The pay
path has a single input — `bolt11`.

Recommended adapters: rust-nostr [`nwc`](https://docs.rs/nwc) for
`NwcWalletService`; [`lnurl-rs`](https://docs.rs/lnurl) for `HttpLnurlResolver`.
`FakeWalletService` and a fake resolver ship in the same crate for tests.

---

## 5. Core flows

### 5.1 Link a wallet

```
User pastes nostr+walletconnect://...  (or scans a QR)
 → backend: buzz-wallet.link(uri)
 → connect to the wallet's relay; get_info + info-event (13194) for capabilities
 → store secret in secure storage (per community)
 → if the wallet exposes a lightning address → publish lud16 in kind:0 on Buzz
 → UI shows "Wallet linked", balance (if supported), available capabilities
```

### 5.2 Show the receiving address

One primitive (`make_invoice`), with an optional static convenience layer:

- **Wallet has a lightning address:** show `alice@getalby.com` (copyable) + QR.
  Static, **asynchronous** receive — the payer does not need you online.
- **Wallet has no lightning address:** Receive tab → `make_invoice(amount)` →
  bolt11 + QR. **Interactive**, one-shot.

This is the one irreducible special case in Lightning (per-payment invoices):
online/interactive vs offline/static receive. It is **isolated behind the
receive/resolver boundary**, so the rest of the code only ever sees "give me a
bolt11".

### 5.3 Receive

No action beyond showing an address/invoice. The user's wallet receives; NWC
notifications (`payment_received`, kind 23196) update balance and history.

### 5.4 Send to a Lightning Address (or a Buzz user with a `lud16`)

```
Pick a recipient (a lud16, or a Buzz user → read lud16 from kind:0 / users.lud16)
 → LnurlResolver: GET lnurlp endpoint → callback → request invoice for N sats
 → VALIDATE: domain/TLS, min/maxSendable, returned invoice amount == requested
 → show CONFIRMATION (amount, recipient, memo)
 → backend: WalletService.pay_invoice(bolt11) -> preimage
 → update history; if in a thread, post KIND_PAYMENT_RECEIPT
```

### 5.5 Payment request in chat (human ↔ human, and identical for agents)

```
B: "Request 500 sats" → client B: make_invoice(500) (or reference lud16)
 → publish KIND_PAYMENT_REQUEST in the channel/DM (h-scoped, p=B)
A sees the pay card "Pay 500 sats to B" → taps Pay → CONFIRM
 → WalletService.pay_invoice -> preimage → publish KIND_PAYMENT_RECEIPT (e=request, preimage)
Thread shows "Paid ✓"; B confirms via B's own wallet (not the posted preimage)
```

There is no separate flow for agents — see below.

---

## 6. Agents that interact with humans

Buzz's distinctive case. In Phase 1 agents **receive and request**; they do not
spend. Crucially, **there is no agent-specific code path** — an agent is a
keypair that posts the same events a human posts. The only difference is a
capability of the wallet behind that identity (receive-only), not a branch in
the payment code.

What an agent can do in Phase 1:

1. **Request payment from a human.** The agent posts `KIND_PAYMENT_REQUEST` (via
   `buzz wallet request`). Examples: "5,000 sats to proceed with this purchase",
   a tip, a service balance. The human sees the pay card, confirms, and pays from
   their linked wallet.
2. **Show a verifiable receiving address.** The agent's owner provisions a
   **receive-only** NWC connection (no spend risk), so the agent can
   `make_invoice` / `lookup_invoice`.
3. **Confirm the incoming payment and continue** — by querying **its own
   wallet** (`lookup_invoice` / `payment_received`), not by trusting a posted
   receipt.
4. **Ask its owner to pay on its behalf.** The agent (which cannot spend) posts a
   request to its owner; the owner (a human) confirms and pays. The key pattern
   for "an agent buys something with human approval".

**Receive-only provisioning.** Extend the env injection in `buzz-acp`
(`build_mcp_servers`, `crates/buzz-acp/src/lib.rs`) to optionally inject a
**receive-only** `BUZZ_NWC_URI` when the owner has configured one. Add
`BUZZ_NWC_URI` to the reserved env keys in
`desktop/src-tauri/src/managed_agents/env_vars.rs` (the user cannot override it)
and to the log scrubbing in `managed_agents/backend.rs`. Provisioning happens
where the agent is created (`desktop/src-tauri/src/commands/agents.rs`, beside
`Keys::generate()` and the auth tag).

**Rendering.** The same pay card / receipt renders whether a human or an agent
posted it — no separate UI. A badge distinguishes "requested by an agent" and
shows the owner (already available via `agent_owner_pubkey`) so the human knows
who is behind it before confirming.

**Hard boundary:** an agent never receives a spend-capable NWC secret in Phase 1.
Autonomous spend is Phase 2 (budget clauses in NIP-OA `conditions` +
a relay `WalletSpend` scope).

---

## 7. Why NWC is a detail (screaming "payments")

The top-level structure names *payments*, not *NWC*. NWC is one adapter behind
`WalletService`. This is what makes the earlier "custodial vs NWC" fork a
non-decision for Phase 1: a future custodial Phase becomes
`CustodialWalletService: WalletService`, and **no use-case, entity, event, or UI
changes**. OCP holds; the fork is deferred at zero cost.

---

## 8. Security model

- **Secret isolation.** The NWC secret lives only in native secure storage +
  native backend. Never in the web/Flutter UI or logs. Reuse `secret_store.rs`
  (off the env-read path) and the existing secret log-scrubbing.
- **Mandatory confirmation** for every send. No silent spend in Phase 1 — doubly
  so when an agent is the one requesting.
- **Budget backstop:** rely on the wallet's own NWC per-connection spend limits.
- **LNURL validation:** domain/TLS, `min`/`maxSendable`, returned invoice amount
  == requested, timeouts; reject non-HTTPS callbacks.
- **Receipt verification** (`SHA256(preimage) == payment_hash`) is available for
  display, but **trust is local** — the authoritative signal is each party's own
  wallet.
- **Abuse:** public-channel requests are spammable → reuse the existing rate-limit
  tiers for `KIND_PAYMENT_REQUEST`.

---

## 9. Testing

- **Unit (domain, `buzz-core`):** URI parsing, LNURL validation, `verify` — no
  network.
- **Use-cases (`buzz-wallet`):** driven entirely by `FakeWalletService` — every
  send/request/verify path tested deterministically, **without a real wallet or
  network**. This is the seam that pays for the `WalletService` boundary.
- **E2E relay:** ingest/fan-out/gating of the two kinds with `h`-scoping
  (`crates/buzz-test-client/tests/`).
- **Desktop E2E:** pay card, receive/send flows via the mock bridge; screenshots
  via `just desktop-screenshot`.
- **NWC end-to-end:** a regtest wallet or a mock NWC service; documented in
  `crates/buzz-cli/TESTING.md`.
- **Mobile / Dart parity:** see [§11](#11-the-dart-duplication).

---

## 10. Build sequence

1. **`buzz-core`** — payment domain types + `verify(preimage, payment_hash)` +
   request validation. *(pure domain, unit-tested, no network)*
2. **`buzz-wallet`** — `WalletService` / `LnurlResolver` ports +
   `FakeWalletService`. *(use-cases tested against the fake — no real wallet)*
3. **Adapters** — `NwcWalletService` (rust-nostr) + `HttpLnurlResolver`.
4. **`buzz-sdk`** builders + kinds in `buzz-core` + thin relay ingest
   (`h`-scoped, no logic).
5. **Tauri commands** (secret in `secret_store.rs`) + community reset.
6. **Desktop UI** (rem-only text, per the zoom rules) + `buzz-cli wallet`.
7. **Agents** — receive-only injection in `buzz-acp` + reserved env. **No new
   code path** — only a different capability.
8. **Mobile** — Dart UI + conformance vectors against the Rust domain.

---

## 11. The Dart duplication

Mobile is pure Flutter (no easy FFI to a Rust crate in this repo). Reimplementing
the NWC **protocol** in Dart is two sources of truth and invites divergence bugs.
Two honest options:

1. **FFI the Rust domain** into Flutter — one source of truth, at a build cost.
2. **Conformance vectors generated from the Rust domain** (preimage verification,
   URI parsing, LNURL cases) that *both* implementations must pass.

Recommendation: ship (2) now, move to (1) if the surface grows. The **UI** may
diverge; the **domain** must not — pin it with shared vectors.

---

## 12. Open decisions

1. **Static address for wallets without a `lud16`:** accept interactive
   (one-shot) receive — simplest, zero server — or host an LNURL-pay proxy on the
   relay that runs `make_invoice` on demand against a user's receive-only NWC
   connection (gives everyone a `pubkey@community` address without custody, but
   adds an HTTP endpoint and a stored limited secret). Recommend: start
   interactive; evaluate the proxy as a 1.5.
2. **Per-community vs global wallet:** proposal is per-community (consistent with
   `nsec` and the community-state reset). Confirm.
3. **NIP-57 zaps:** include in Phase 1 (tip on a message) or defer? Buildable on
   top of the payment-request primitive.
4. **NWC / LNURL libraries:** rust-nostr `nwc` + `lnurl-rs` (recommended) vs a
   hand-rolled implementation.

---

## References

- [NIP-47 — Nostr Wallet Connect](https://github.com/nostr-protocol/nips/blob/master/47.md)
- [NIP-57 — Lightning Zaps](https://github.com/nostr-protocol/nips/blob/master/57.md)
- [LUD-16 — Lightning Addresses](https://github.com/lnurl/luds/blob/luds/16.md)
- [LUD-06 — LNURL-pay](https://github.com/lnurl/luds/blob/luds/06.md)
- `buzz-core/src/kind.rs` — event kind registry
- `buzz-sdk/src/nip_oa.rs` — owner→agent authorization (the Phase 2 spend-budget seam)
