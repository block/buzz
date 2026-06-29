#!/usr/bin/env python3
"""Derive the loopback community hosts to seed for local dev.

The relay uses row-zero host binding: it resolves a request's community from the
Host header and fails closed (404) when that authority is not a `communities`
row. Local dev tooling hits the relay under several loopback authorities
(``localhost``/``127.0.0.1``, with and without the port), and under host binding
those are *distinct* keys -- so we seed each one to avoid a fail-closed 404 when
one client uses an alternate authority.

Reads RELAY_URL from the environment (default ``ws://localhost:3000``) and prints
the authorities to seed, one per line, normalized the same way the relay
normalizes a community host. seed-local-community.sh feeds these to
seed-communities.sql.
"""

import os
from urllib.parse import urlparse


def derive_hosts(relay_url):
    """Return the ordered, de-duplicated list of authorities to seed."""
    parsed = urlparse(relay_url)
    host = (parsed.hostname or "").rstrip(".").lower()
    port = parsed.port
    scheme = parsed.scheme.lower()

    def authority(h):
        if not h:
            return ""
        display_host = f"[{h}]" if ":" in h and not h.startswith("[") else h
        default_port = (scheme == "ws" and port == 80) or (scheme == "wss" and port == 443)
        if port and not default_port:
            return f"{display_host}:{port}"
        return display_host

    hosts = []
    primary = authority(host)
    if primary:
        hosts.append(primary)

    # Non-loopback deployments seed only RELAY_URL's authority; loopback dev
    # additionally seeds both localhost and 127.0.0.1, with and without port.
    if host in {"localhost", "127.0.0.1"}:
        hosts.extend(["localhost", "127.0.0.1"])
        if port:
            hosts.extend([f"localhost:{port}", f"127.0.0.1:{port}"])

    seen = []
    for h in hosts:
        if h and h not in seen:
            seen.append(h)
    return seen


def main():
    relay_url = os.environ.get("RELAY_URL", "ws://localhost:3000")
    hosts = derive_hosts(relay_url)
    if not hosts:
        raise SystemExit("could not derive a host from RELAY_URL")
    for h in hosts:
        print(h)


if __name__ == "__main__":
    main()
