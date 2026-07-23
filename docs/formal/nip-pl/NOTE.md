---
title: "NIP-PL Formal Models: lease acceptance and stateful gateway authority"
tags: [nostr, nip-pl, push-notifications, formal-model, buzz]
status: active
created: 2026-07-11
---

# NIP-PL formal pressure test

These bounded executable models cover two distinct shipped contracts. They do not model the not-yet-shipped relay matcher/worker or any removed wake-grant protocol.

## Run

```bash
python3 acceptance.py
python3 mutation_test.py
python3 delivery.py
python3 delivery_mutation.py
python3 fixed_payload.py
python3 fixed_payload_mutation.py
```

## Lease acceptance

`acceptance.py` explores all 5040 orderings of one address's active, revoke, reactivate, replay, NIP-01-tie, high-generation/old-created-at, and high-created-at/stale-generation candidates. It checks no resurrection, monotone watermarks, no watermark poisoning, agreement between stored and effective state, and replay-window safety. `mutation_test.py` independently drops the NIP-01 and generation clauses and requires both mutants to produce witnesses.

## Stateful public gateway

`delivery.py` models the authority actually shipped by the public gateway: relay signer confinement; installation/delegation epoch, generation, and expiry; atomic replay/quota admission; revocation ordering; custody; terminal request burn versus transient release; and the exact constant APNs body. `delivery_mutation.py` requires signer, epoch, terminal-burn, quota-refund, and fixed-body mutants to be detected.

## Fixed payload

`fixed_payload.py` exhaustively varies relay-controlled and gateway-state inputs and requires the APNs body to remain byte-identical to the normative constant. `fixed_payload_mutation.py` injects each prohibited input category and requires every mutation to be caught.

## Honest limits

The models enumerate bounded abstract transitions, not SQL schedules or network behavior. Real PostgreSQL race/FK/retention tests validate the implementation separately. A crash after APNs accepts but before disposition persistence remains intentionally at-least-once; request expiry bounds the resulting replay reservation. Constant payload prevents content disclosure but cannot hide wake timing or frequency.
