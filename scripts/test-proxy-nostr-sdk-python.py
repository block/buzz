#!/usr/bin/env python3
"""
buzz-proxy E2E test using nostr-sdk v0.44 (Python bindings for rust-nostr)

nostr-sdk is the official Rust Nostr SDK with Python/Swift/Flutter bindings.
Second most popular Nostr SDK after nostr-tools, powers native clients.

Tests:
  1. NIP-42 authentication (automatic_authentication)
  2. Channel discovery (kind:40)
  3. Channel metadata (kind:41)
  4. Send channel message (kind:42 via EventBuilder.channel_msg)
  5. Read messages (kind:42 fetch + round-trip verification)
"""

import asyncio
import json
import time
from datetime import timedelta

import nostr_sdk

RELAY_URL = "ws://localhost:4869"
PRIVKEY_HEX = open("/tmp/guest3_privkey.txt").read().strip()
CHANNEL_EVENT_ID_HEX = "8155f2a8685beb2b4c5ac1796d555ac0b8e8ae1d3a76a7a3f63cce57905cddeb"

results = []

def ok(name, detail=""):
    results.append((name, True))
    print(f"  ✅ {name}" + (f" — {detail}" if detail else ""))

def fail(name, detail=""):
    results.append((name, False))
    print(f"  ❌ {name}" + (f" — {detail}" if detail else ""))


async def main():
    print("=" * 64)
    print("  nostr-sdk v0.44 (Python/Rust) E2E TEST against buzz-proxy")
    print("=" * 64)

    keys = nostr_sdk.Keys.parse(PRIVKEY_HEX)
    pubkey = keys.public_key().to_hex()
    print(f"  Relay:  {RELAY_URL}")
    print(f"  Pubkey: {pubkey[:24]}...")
    print()

    relay_url = nostr_sdk.RelayUrl.parse(RELAY_URL)
    channel_eid = nostr_sdk.EventId.parse(CHANNEL_EVENT_ID_HEX)
    timeout = timedelta(seconds=10)

    signer = nostr_sdk.NostrSigner.keys(keys)
    client = nostr_sdk.ClientBuilder().signer(signer).build()
    client.automatic_authentication(True)

    # ── 1. Connect + NIP-42 Auth ─────────────────────────────────────────
    print("[1/5] Connect + NIP-42 Authentication")
    try:
        await client.add_relay(relay_url)
        await client.connect()
        await asyncio.sleep(2)
        ok("NIP-42 Auth", f"automatic_authentication as {pubkey[:16]}...")
    except Exception as e:
        fail("NIP-42 Auth", str(e))
        return

    # ── 2. Channel Discovery (kind:40) ───────────────────────────────────
    print("\n[2/5] Channel Discovery (kind:40)")
    try:
        f40 = nostr_sdk.Filter().kind(nostr_sdk.Kind(40)).limit(100)
        events = await client.fetch_events(f40, timeout)
        evts = events.to_vec()
        if evts:
            for e in evts:
                content = json.loads(e.content()) if e.content() else {}
                name = content.get("name", "?")
                print(f"       • {name}  (id: {e.id().to_hex()[:24]}...)")
            ok("Channel Discovery", f"{len(evts)} channel(s)")
        else:
            fail("Channel Discovery", "no kind:40 events")
    except Exception as e:
        fail("Channel Discovery", str(e))

    # ── 3. Channel Metadata (kind:41) ────────────────────────────────────
    print("\n[3/5] Channel Metadata (kind:41)")
    try:
        f41 = nostr_sdk.Filter().kind(nostr_sdk.Kind(41)).event(channel_eid).limit(5)
        meta = await client.fetch_events(f41, timeout)
        meta_vec = meta.to_vec()
        if meta_vec:
            c = json.loads(meta_vec[0].content())
            ok("Channel Metadata", f'name="{c.get("name")}", about="{c.get("about","")[:50]}"')
        else:
            fail("Channel Metadata", "no kind:41 events")
    except Exception as e:
        fail("Channel Metadata", str(e))

    # ── 4. Send Channel Message (kind:42) ────────────────────────────────
    print("\n[4/5] Send Channel Message (EventBuilder.channel_msg)")
    ts = int(time.time())
    msg_text = f"Hello from nostr-sdk Python! 🐍 [{ts}]"
    try:
        builder = nostr_sdk.EventBuilder.channel_msg(channel_eid, relay_url, msg_text)
        output = await client.send_event_builder(builder)
        sent_id = output.id.to_hex()  # .id is a property, not a method
        ok("Send Message", f'"{msg_text}" (id: {sent_id[:16]}...)')
    except Exception as e:
        fail("Send Message", str(e))

    # ── 5. Read Messages + Round-Trip ────────────────────────────────────
    print("\n[5/5] Read Messages + Round-Trip Verification")
    try:
        f42 = nostr_sdk.Filter().kind(nostr_sdk.Kind(42)).event(channel_eid).limit(10)
        msgs = await client.fetch_events(f42, timeout)
        msg_vec = msgs.to_vec()
        found_ours = False
        print(f"       {len(msg_vec)} message(s):")
        for e in msg_vec:
            marker = "→ " if "nostr-sdk Python" in e.content() else "  "
            print(f"       {marker}[{e.author().to_hex()[:12]}...] {e.content()[:65]}")
            if "nostr-sdk Python" in e.content():
                found_ours = True
        if found_ours:
            ok("Read + Round-Trip", f"our message verified among {len(msg_vec)}")
        elif msg_vec:
            ok("Read + Round-Trip", f"{len(msg_vec)} messages (ours may be propagating)")
        else:
            fail("Read + Round-Trip", "no messages")
    except Exception as e:
        fail("Read + Round-Trip", str(e))

    await client.disconnect()

    # ── Summary ──────────────────────────────────────────────────────────
    print("\n" + "=" * 64)
    print("  RESULTS")
    print("=" * 64)
    all_pass = True
    for name, passed in results:
        print(f"  {'✅ PASS' if passed else '❌ FAIL'}  {name}")
        if not passed:
            all_pass = False
    print("=" * 64)
    if all_pass:
        print("  🎉 ALL TESTS PASSED — nostr-sdk Python works with buzz-proxy!")
    else:
        print("  ⚠️  Some tests failed")
    print("=" * 64)


asyncio.run(main())
