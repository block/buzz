#!/usr/bin/env python3
"""Live Nostr connection to the Buzz relay for any fleet agent.

Parameterized version of buzz_listen.py — any agent can run a live listener
by specifying its name and key from the fleet keyring.

NIP-42 auth, then poll-subscribe to kind-9 stream messages. Emits one line
per event addressed to this agent (p-tag mention or @name text). Reconnects
on drop. Writes events to stdout and to /tmp/buzz-<name>-events.log.

Usage:
    python3 scripts/buzz_fleet_listen.py --agent devin-local
    python3 scripts/buzz_fleet_listen.py --agent hermes --relay ws://127.0.0.1:3030

The agent name must match a BUZZ_<NAME>_KEY / BUZZ_<NAME>_PUB pair in
.fleet_keys.env (name uppercased, hyphens to underscores).
"""
import argparse
import asyncio
import hashlib
import json
import os
import re
import sys
import time
from pathlib import Path

from coincurve import PrivateKey
import websockets

DEFAULT_RELAY = "ws://127.0.0.1:3030"
DEFAULT_KEYFILE = str(Path(__file__).resolve().parent.parent / ".fleet_keys.env")
POLL_INTERVAL = 5  # seconds between subscription refreshes


def load_agent_key(agent_name: str, keyfile: str):
    """Load keypair for the given agent from the fleet keyring."""
    var_prefix = agent_name.upper().replace("-", "_")
    text = open(keyfile).read()
    sk_match = re.search(rf"^BUZZ_{var_prefix}_KEY=(\S+)$", text, re.M)
    pub_match = re.search(rf"^BUZZ_{var_prefix}_PUB=(\S+)$", text, re.M)
    if not sk_match or not pub_match:
        available = re.findall(r"^BUZZ_(\w+)_KEY=", text, re.M)
        print(f"ERROR: agent '{agent_name}' not found in {keyfile}", file=sys.stderr)
        print(f"  Available agents: {', '.join(available)}", file=sys.stderr)
        sys.exit(1)
    return sk_match.group(1), pub_match.group(1)


def sign_event(sk_hex: str, pub_hex: str, kind: int, tags, content: str) -> dict:
    created = int(time.time())
    payload = json.dumps([0, pub_hex, created, kind, tags, content],
                         separators=(",", ":"), ensure_ascii=False)
    eid = hashlib.sha256(payload.encode()).hexdigest()
    priv = PrivateKey(bytes.fromhex(sk_hex))
    sig = priv.sign_schnorr(bytes.fromhex(eid), aux_randomness=b"\x00" * 32)
    return {"id": eid, "pubkey": pub_hex, "created_at": created, "kind": kind,
            "tags": tags, "content": content, "sig": sig.hex()}


def is_for_me(ev: dict, mypub: str, my_name: str) -> bool:
    if ev.get("pubkey") == mypub:
        return False
    # p-tag mention
    if any(t[0] == "p" and len(t) > 1 and t[1] == mypub for t in ev.get("tags", [])):
        return True
    # @name text mention
    return my_name.lower() in (ev.get("content") or "").lower()


def emit(ev: dict, events_file: str):
    who = ev.get("pubkey", "")[:8]
    body = (ev.get("content") or "").replace("\n", " ")[:300]
    line = f"buzz mention [{who}]: {body}"
    print(line, flush=True)
    try:
        with open(events_file, "a") as f:
            f.write(line + "\n")
    except OSError:
        pass


async def listen(agent_name: str, sk: str, mypub: str, relay: str, events_file: str):
    since = int(time.time())
    seen_ids = set()
    print(f"starting: {agent_name} ({mypub[:16]}...), polling every {POLL_INTERVAL}s", flush=True)
    print(f"  relay: {relay}", flush=True)
    print(f"  events log: {events_file}", flush=True)

    while True:
        try:
            async with websockets.connect(relay, ping_interval=20) as ws:
                # Send initial REQ to trigger AUTH challenge
                await ws.send(json.dumps(["REQ", "boot", {"kinds": [9], "limit": 1}]))
                authed = False

                while True:
                    try:
                        raw = await asyncio.wait_for(ws.recv(), timeout=POLL_INTERVAL)
                    except asyncio.TimeoutError:
                        break

                    msg = json.loads(raw)
                    typ = msg[0]

                    if typ == "AUTH" and not authed:
                        challenge = msg[1]
                        auth_ev = sign_event(sk, mypub, 22242,
                                             [["relay", relay], ["challenge", challenge]], "")
                        await ws.send(json.dumps(["AUTH", auth_ev]))
                        await ws.send(json.dumps(["REQ", "live", {"kinds": [9], "since": since}]))
                        authed = True
                        print(f"connected: authed as {agent_name}, subscription open", flush=True)

                    elif typ == "EVENT":
                        ev = msg[2]
                        eid = ev.get("id", "")
                        if eid in seen_ids:
                            continue
                        seen_ids.add(eid)
                        ct = ev.get("created_at", 0)
                        if ct > since:
                            since = ct
                        if is_for_me(ev, mypub, agent_name):
                            emit(ev, events_file)

                    elif typ == "EOSE":
                        pass

                    elif typ == "CLOSED" and msg[1] == "live":
                        print(f"subscription closed: {str(msg[2:])[:120]}", flush=True)
                        break

                    elif typ == "OK" and len(msg) > 2 and not msg[2]:
                        print(f"relay rejected: {msg}", flush=True)
                        return

            await asyncio.sleep(1)

        except (websockets.ConnectionClosed, ConnectionRefusedError, OSError) as e:
            print(f"relay connection lost ({type(e).__name__}); reconnecting in 15s", flush=True)
            await asyncio.sleep(15)


def main():
    parser = argparse.ArgumentParser(description="Buzz fleet live listener for any agent")
    parser.add_argument("--agent", required=True, help="Agent name (e.g. devin-local, hermes)")
    parser.add_argument("--relay", default=DEFAULT_RELAY, help=f"Relay URL (default: {DEFAULT_RELAY})")
    parser.add_argument("--keyfile", default=DEFAULT_KEYFILE, help="Path to .fleet_keys.env")
    parser.add_argument("--events-file", default=None, help="Path to events log file")
    args = parser.parse_args()

    sk, mypub = load_agent_key(args.agent, args.keyfile)
    events_file = args.events_file or f"/tmp/buzz-{args.agent}-events.log"

    try:
        os.truncate(events_file, 0)
    except (OSError, FileNotFoundError):
        pass

    asyncio.run(listen(args.agent, sk, mypub, args.relay, events_file))


if __name__ == "__main__":
    main()
