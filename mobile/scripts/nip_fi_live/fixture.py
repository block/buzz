#!/usr/bin/env python3
"""Fixture for the mobile NIP-FI live check: a real, enforcing relay.

Starts, all on loopback and all ephemeral:

* a real `buzz-relay` with `BUZZ_NIP_FI_MODE=enforce` on a freshly reset
  database, serving two communities (one per `Host`);
* a test issuer: an ES256 key generated at start-up, its JWKS on plain HTTP
  (`jwks_transport: "mesh"`), and a fake Builderlab account API that mints
  assertions the way the real one does;
* a TLS front with a throwaway CA, routing by `Host` to the relay or the
  fake account API. WebSocket upgrades pass through as raw bytes.

It then writes `fixture.json` into the work directory and waits for SIGTERM.
On shutdown it records whether the relay was still alive (a crashed relay is
a failed check, never a pass) in `fixture-result.json`.

Nothing here is a credential: the issuer key, the CA, the TLS key and the test
member key are generated per run and live only in the work directory.

Requires the `cryptography` package (`uv run --with cryptography ...`), plus
`psql` and the repo's hermit `bin/pgschema` for the database reset.
"""

import argparse
import asyncio
import base64
import datetime
import hashlib
import hmac
import json
import os
import re
import secrets
import signal
import socket
import ssl
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.hazmat.primitives.asymmetric.utils import decode_dss_signature
from cryptography.x509.oid import NameOID

REPO_ROOT = Path(__file__).resolve().parents[3]

# The names the app sees. `*.localhost` is not resolved by the app under test:
# the Dart test maps these names to the TLS front (see `nip_fi_live_fixture.dart`),
# so the app's own relay-URL checks run unchanged on `https://<name>` URLs.
RELAY_HOST = "relay.nipfi.localhost"
SECOND_HOST = "second.nipfi.localhost"
ACCOUNT_HOST = "account.nipfi.localhost"
ISSUER = f"https://{ACCOUNT_HOST}"
OPERATOR_SUBJECT = "test-operator"
SESSION_CREDENTIAL = "c" * 64
SUBJECT = "nip-fi-live-subject"


# ── Nostr key helpers (nsec bech32, x-only public key) ───────────────────────

_BECH32 = "qpzry9x8gf2tvdw0s3jn54khce6mua7l"


def _bech32_polymod(values):
    generator = [0x3B6A57B2, 0x26508E6D, 0x1EA119FA, 0x3D4233DD, 0x2A1462B3]
    chk = 1
    for value in values:
        top = chk >> 25
        chk = (chk & 0x1FFFFFF) << 5 ^ value
        for i in range(5):
            chk ^= generator[i] if (top >> i) & 1 else 0
    return chk


def _bech32_encode(hrp, data: bytes) -> str:
    bits = 0
    acc = 0
    values = []
    for byte in data:
        acc = (acc << 8) | byte
        bits += 8
        while bits >= 5:
            bits -= 5
            values.append((acc >> bits) & 31)
    if bits:
        values.append((acc << (5 - bits)) & 31)
    hrp_expanded = [ord(c) >> 5 for c in hrp] + [0] + [ord(c) & 31 for c in hrp]
    polymod = _bech32_polymod(hrp_expanded + values + [0] * 6) ^ 1
    checksum = [(polymod >> 5 * (5 - i)) & 31 for i in range(6)]
    return hrp + "1" + "".join(_BECH32[v] for v in values + checksum)


def new_member_key():
    secret = secrets.token_bytes(32)
    key = ec.derive_private_key(int.from_bytes(secret, "big"), ec.SECP256K1())
    x = key.public_key().public_numbers().x
    return _bech32_encode("nsec", secret), x.to_bytes(32, "big").hex()


# ── Issuer: ES256 key, JWKS, assertion minting ───────────────────────────────


def _b64url(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode()


class Issuer:
    def __init__(self, lifetime_secs: int):
        self.lifetime_secs = lifetime_secs
        self.key = ec.generate_private_key(ec.SECP256R1())
        self.kid = "nip-fi-live-" + secrets.token_hex(4)
        # Every assertion minted, for the oracle's expiry-path precondition.
        self.mints = []

    def jwks(self):
        numbers = self.key.public_key().public_numbers()
        return {
            "keys": [
                {
                    "kty": "EC",
                    "crv": "P-256",
                    "alg": "ES256",
                    "use": "sig",
                    "kid": self.kid,
                    "x": _b64url(numbers.x.to_bytes(32, "big")),
                    "y": _b64url(numbers.y.to_bytes(32, "big")),
                }
            ]
        }

    def mint(self, audience: str, pubkey_hex: str):
        now = int(time.time())
        header = {"typ": "nip-fi+jwt", "alg": "ES256", "kid": self.kid}
        claims = {
            "iss": ISSUER,
            "sub": SUBJECT,
            "nostr_pubkey": pubkey_hex,
            "aud": audience,
            "iat": now,
            "exp": now + self.lifetime_secs,
        }
        signing_input = (
            _b64url(json.dumps(header, separators=(",", ":")).encode())
            + "."
            + _b64url(json.dumps(claims, separators=(",", ":")).encode())
        )
        self.mints.append(
            {
                "t": time.time(),
                "aud": audience,
                "iat": claims["iat"],
                "exp": claims["exp"],
            }
        )
        der = self.key.sign(signing_input.encode(), ec.ECDSA(hashes.SHA256()))
        r, s = decode_dss_signature(der)
        signature = r.to_bytes(32, "big") + s.to_bytes(32, "big")
        return signing_input + "." + _b64url(signature), claims["exp"]


class _Loopback6Server(ThreadingHTTPServer):
    address_family = socket.AF_INET6


def serve(handler_class, port=0):
    server = ThreadingHTTPServer(("127.0.0.1", port), handler_class)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def serve_dual_loopback(handler_class):
    """The same server on 127.0.0.1 and ::1, on one port.

    The relay resolves the JWKS host name (`localhost`) and pins the first
    address it gets, which is ::1 on some hosts and 127.0.0.1 on others.
    Returns the IPv4 server, whose `server_port` is the shared port, and every
    server started, for shutdown.
    """
    for _ in range(50):
        try:
            six = _Loopback6Server(("::1", 0), handler_class)
        except OSError:
            # No IPv6 loopback: the name can only resolve to IPv4.
            return [serve(handler_class)]
        try:
            four = ThreadingHTTPServer(("127.0.0.1", six.server_port), handler_class)
        except OSError:
            six.server_close()
            continue
        for server in (six, four):
            threading.Thread(target=server.serve_forever, daemon=True).start()
        return [four, six]
    raise SystemExit("no free port on both loopback addresses for the JWKS server")


def jwks_handler(issuer: Issuer):
    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            if self.path != "/jwks.json":
                self.send_error(404)
                return
            body = json.dumps(issuer.jwks()).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *_args):
            pass

    return Handler


def account_handler(issuer: Issuer, pubkey_hex: str, audiences: set, requests: list):
    """The Builderlab account API calls the app makes on this path, and no
    others: an unexpected path is a 404, so API drift fails loudly."""

    expires_at = (
        datetime.datetime.now(datetime.UTC) + datetime.timedelta(hours=4)
    ).strftime("%Y-%m-%dT%H:%M:%SZ")

    class Handler(BaseHTTPRequestHandler):
        def _json(self, status, value):
            body = json.dumps(value).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def _body(self):
            length = int(self.headers.get("Content-Length") or 0)
            raw = self.rfile.read(length) if length else b""
            return json.loads(raw) if raw else {}

        def _authenticated(self):
            return self.headers.get("X-BB-Session-Credential") == SESSION_CREDENTIAL

        def do_GET(self):
            requests.append(("GET", self.path))
            if self.path == "/api/goose/v1/auth/me" and self._authenticated():
                self._json(
                    200,
                    {
                        "subject": SUBJECT,
                        "expires_at": expires_at,
                        "account_status": "ACCOUNT_STATUS_ACTIVE",
                    },
                )
            else:
                self._json(404, {"error": "unexpected path " + self.path})

        def do_POST(self):
            requests.append(("POST", self.path))
            body = self._body()
            if self.path == "/api/goose/v1/auth/login/exchange":
                if not body.get("code") or not body.get("code_verifier"):
                    self._json(400, {"error": "bad exchange"})
                    return
                self._json(
                    200,
                    {
                        "session_credential": SESSION_CREDENTIAL,
                        "expires_at": expires_at,
                    },
                )
            elif not self._authenticated():
                self._json(401, {"error": "no session"})
            elif self.path == "/api/goose/v1/buzz/nostr-identities/current":
                self._json(200, {"identity": {"pubkey_hex": pubkey_hex}})
            elif self.path == "/api/goose/v1/buzz/identity/assertions":
                relay = urllib.parse.urlparse(body.get("relay_url", ""))
                audience = f"https://{relay.hostname}"
                if (
                    audience not in audiences
                    or body.get("nostr_pubkey") != pubkey_hex
                    or not (self.headers.get("Authorization") or "").startswith(
                        "Nostr "
                    )
                ):
                    self._json(403, {"error": "refused"})
                    return
                token, expiry = issuer.mint(audience, pubkey_hex)
                self._json(
                    200,
                    {
                        "assertion": token,
                        "expires_at": expiry,
                        "nostr_pubkey": pubkey_hex,
                    },
                )
            else:
                self._json(404, {"error": "unexpected path " + self.path})

        def log_message(self, *_args):
            pass

    return Handler


# ── TLS: throwaway CA, leaf certificate, Host-routing front ──────────────────


def make_tls(workdir: Path):
    def name(common_name):
        return x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, common_name)])

    now = datetime.datetime.now(datetime.UTC)
    ca_key = ec.generate_private_key(ec.SECP256R1())
    ca_cert = (
        x509.CertificateBuilder()
        .subject_name(name("NIP-FI live check CA"))
        .issuer_name(name("NIP-FI live check CA"))
        .public_key(ca_key.public_key())
        .serial_number(x509.random_serial_number())
        .not_valid_before(now - datetime.timedelta(minutes=5))
        .not_valid_after(now + datetime.timedelta(days=1))
        .add_extension(x509.BasicConstraints(ca=True, path_length=0), critical=True)
        .add_extension(
            x509.KeyUsage(
                digital_signature=False,
                content_commitment=False,
                key_encipherment=False,
                data_encipherment=False,
                key_agreement=False,
                key_cert_sign=True,
                crl_sign=True,
                encipher_only=False,
                decipher_only=False,
            ),
            critical=True,
        )
        .add_extension(
            x509.SubjectKeyIdentifier.from_public_key(ca_key.public_key()),
            critical=False,
        )
        .sign(ca_key, hashes.SHA256())
    )
    leaf_key = ec.generate_private_key(ec.SECP256R1())
    leaf_cert = (
        x509.CertificateBuilder()
        .subject_name(name(RELAY_HOST))
        .issuer_name(ca_cert.subject)
        .public_key(leaf_key.public_key())
        .serial_number(x509.random_serial_number())
        .not_valid_before(now - datetime.timedelta(minutes=5))
        .not_valid_after(now + datetime.timedelta(days=1))
        .add_extension(
            x509.SubjectAlternativeName(
                [x509.DNSName(h) for h in (RELAY_HOST, SECOND_HOST, ACCOUNT_HOST)]
            ),
            critical=False,
        )
        .add_extension(x509.BasicConstraints(ca=False, path_length=None), critical=True)
        .add_extension(
            x509.KeyUsage(
                digital_signature=True,
                content_commitment=False,
                key_encipherment=False,
                data_encipherment=False,
                key_agreement=False,
                key_cert_sign=False,
                crl_sign=False,
                encipher_only=False,
                decipher_only=False,
            ),
            critical=True,
        )
        .add_extension(
            x509.ExtendedKeyUsage([x509.oid.ExtendedKeyUsageOID.SERVER_AUTH]),
            critical=False,
        )
        .add_extension(
            x509.SubjectKeyIdentifier.from_public_key(leaf_key.public_key()),
            critical=False,
        )
        .add_extension(
            x509.AuthorityKeyIdentifier.from_issuer_public_key(ca_key.public_key()),
            critical=False,
        )
        .sign(ca_key, hashes.SHA256())
    )
    ca_pem = workdir / "ca.pem"
    ca_pem.write_bytes(ca_cert.public_bytes(serialization.Encoding.PEM))
    chain = workdir / "front-chain.pem"
    key = workdir / "front-key.pem"
    chain.write_bytes(leaf_cert.public_bytes(serialization.Encoding.PEM))
    key.write_bytes(
        leaf_key.private_bytes(
            serialization.Encoding.PEM,
            serialization.PrivateFormat.PKCS8,
            serialization.NoEncryption(),
        )
    )
    key.chmod(0o600)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(chain, key)
    return ca_pem, context


class TlsFront:
    """Terminates TLS and pipes bytes to a backend chosen by the first
    request's `Host`. The app's HTTP client keeps one `Host` per connection,
    so one routing decision per connection is enough."""

    def __init__(self, context, backends: dict):
        self.context = context
        self.backends = backends
        self.port = None
        self.loop = asyncio.new_event_loop()
        ready = threading.Event()
        threading.Thread(target=self._run, args=(ready,), daemon=True).start()
        ready.wait()

    def _run(self, ready):
        asyncio.set_event_loop(self.loop)
        server = self.loop.run_until_complete(
            asyncio.start_server(self._handle, "127.0.0.1", 0, ssl=self.context)
        )
        self.port = server.sockets[0].getsockname()[1]
        ready.set()
        self.loop.run_forever()

    async def _handle(self, reader, writer):
        try:
            head = await reader.readuntil(b"\r\n\r\n")
            host = ""
            for line in head.split(b"\r\n")[1:]:
                if line.lower().startswith(b"host:"):
                    host = line[5:].strip().decode().split(":")[0].lower()
            backend = self.backends.get(host)
            if backend is None:
                writer.write(
                    b"HTTP/1.1 421 Misdirected Request\r\nContent-Length: 0\r\n\r\n"
                )
                await writer.drain()
                return
            up_reader, up_writer = await asyncio.open_connection("127.0.0.1", backend)
            up_writer.write(head)
            await up_writer.drain()
            await asyncio.gather(
                self._pipe(reader, up_writer), self._pipe(up_reader, writer)
            )
        except (
            asyncio.IncompleteReadError,
            ConnectionError,
            ssl.SSLError,
            OSError,
        ) as error:
            print(
                f"tls front: connection ended: {error!r}", file=sys.stderr, flush=True
            )
        finally:
            writer.close()

    @staticmethod
    async def _pipe(source, sink):
        try:
            while data := await source.read(65536):
                sink.write(data)
                await sink.drain()
        except (ConnectionError, ssl.SSLError, OSError):
            pass
        finally:
            sink.close()


# ── Database, object store, relay ────────────────────────────────────────────


def free_port():
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


LOOPBACK_HOSTS = {"localhost", "127.0.0.1", "::1"}


def check_throwaway_database(database_url: str):
    """Refuse to reset anything that is not clearly a throwaway database.

    The reset drops the `public` schema, so the target must be on this machine
    and its name must say it is for this check. Returns the parsed URL.
    """
    parsed = urllib.parse.urlparse(database_url)
    name = parsed.path.lstrip("/")
    if (parsed.hostname or "localhost") not in LOOPBACK_HOSTS:
        sys.exit(
            f"refusing to reset a database on {parsed.hostname}: "
            "the NIP-FI check only resets a database on this machine"
        )
    if not re.fullmatch(r"[A-Za-z0-9_]*nip_fi[A-Za-z0-9_]*", name):
        sys.exit(
            f"refusing to reset database `{name}`: its name must contain "
            "`nip_fi` (the reset drops the whole public schema). "
            "Set NIP_FI_DATABASE_URL to a dedicated database such as "
            "`postgres://<user>:<password>@localhost:5432/buzz_nip_fi`."
        )
    return parsed


def ensure_database(parsed, psql: str):
    """Creates the throwaway database when it does not exist yet."""
    name = parsed.path.lstrip("/")
    env = {
        **os.environ,
        "PGHOST": parsed.hostname or "localhost",
        "PGPORT": str(parsed.port or 5432),
        "PGUSER": urllib.parse.unquote(parsed.username or ""),
        "PGPASSWORD": urllib.parse.unquote(parsed.password or ""),
        "PGDATABASE": "postgres",
    }
    exists = subprocess.run(
        [
            psql,
            "-v",
            "ON_ERROR_STOP=1",
            "-tAc",
            f"SELECT 1 FROM pg_database WHERE datname = '{name}'",
        ],
        check=True,
        env=env,
        capture_output=True,
        text=True,
    ).stdout.strip()
    if not exists:
        subprocess.run(
            [psql, "-v", "ON_ERROR_STOP=1", "-qc", f'CREATE DATABASE "{name}"'],
            check=True,
            env=env,
        )


def reset_database(database_url: str, psql: str):
    parsed = check_throwaway_database(database_url)
    ensure_database(parsed, psql)
    env = {
        **os.environ,
        "PGHOST": parsed.hostname or "localhost",
        "PGPORT": str(parsed.port or 5432),
        "PGUSER": urllib.parse.unquote(parsed.username or ""),
        "PGPASSWORD": urllib.parse.unquote(parsed.password or ""),
        "PGDATABASE": parsed.path.lstrip("/"),
    }
    env.update(
        PGSCHEMA_PLAN_HOST=env["PGHOST"],
        PGSCHEMA_PLAN_PORT=env["PGPORT"],
        PGSCHEMA_PLAN_DB=env["PGDATABASE"],
        PGSCHEMA_PLAN_USER=env["PGUSER"],
        PGSCHEMA_PLAN_PASSWORD=env["PGPASSWORD"],
    )
    subprocess.run(
        [
            psql,
            "-v",
            "ON_ERROR_STOP=1",
            "-qc",
            "DROP SCHEMA public CASCADE; CREATE SCHEMA public;",
        ],
        check=True,
        env=env,
    )
    subprocess.run(
        [
            str(REPO_ROOT / "bin/pgschema"),
            "apply",
            "--file",
            "schema/schema.sql",
            "--auto-approve",
        ],
        check=True,
        env=env,
        cwd=REPO_ROOT,
    )
    with open(REPO_ROOT / "scripts/reconcile-schema-after-pgschema.sql") as sql:
        subprocess.run(
            [psql, "-v", "ON_ERROR_STOP=1", "-q"], check=True, env=env, stdin=sql
        )
    return env


def seed(env, psql: str, member_pubkey: str):
    """One open channel per community, with the test member in it."""
    system = "0" * 64
    ids = {}
    statements = []
    for label, host in (("relay", RELAY_HOST), ("second", SECOND_HOST)):
        channel = str(uuid.uuid5(uuid.NAMESPACE_DNS, f"buzz.nip-fi-live.{label}"))
        ids[label] = {"channel": channel}
        statements.append(
            f"INSERT INTO communities (host) VALUES ('{host}') ON CONFLICT (lower(host)) DO NOTHING;"
        )
        statements.append(
            f"""INSERT INTO channels (community_id, id, name, channel_type, visibility, description, created_by, topic_required)
SELECT id, '{channel}', 'general', 'stream', 'open', 'NIP-FI live check', decode('{system}','hex'), false
FROM communities WHERE lower(host) = '{host}';"""
        )
        statements.append(
            f"""INSERT INTO channel_members (community_id, channel_id, pubkey, role, invited_by)
SELECT id, '{channel}', decode('{member_pubkey}','hex'), 'member', decode('{system}','hex')
FROM communities WHERE lower(host) = '{host}';"""
        )
    subprocess.run(
        [psql, "-v", "ON_ERROR_STOP=1", "-q"],
        check=True,
        env=env,
        input="\n".join(statements),
        text=True,
    )
    for label, host in (("relay", RELAY_HOST), ("second", SECOND_HOST)):
        out = subprocess.run(
            [
                psql,
                "-qtA",
                "-c",
                f"SELECT id FROM communities WHERE lower(host) = '{host}'",
            ],
            check=True,
            env=env,
            capture_output=True,
            text=True,
        )
        ids[label]["community"] = out.stdout.strip()
    return ids


def ensure_bucket(
    endpoint: str, access: str, secret: str, bucket: str, region="us-east-1"
):
    """Create the media bucket with a SigV4 PUT; an existing bucket is fine."""
    parsed = urllib.parse.urlparse(endpoint)
    now = datetime.datetime.now(datetime.UTC)
    amz_date = now.strftime("%Y%m%dT%H%M%SZ")
    date = now.strftime("%Y%m%d")
    payload_hash = hashlib.sha256(b"").hexdigest()
    host = parsed.netloc
    canonical = (
        f"PUT\n/{bucket}\n\nhost:{host}\nx-amz-content-sha256:{payload_hash}\n"
        f"x-amz-date:{amz_date}\n\nhost;x-amz-content-sha256;x-amz-date\n{payload_hash}"
    )
    scope = f"{date}/{region}/s3/aws4_request"
    to_sign = f"AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{hashlib.sha256(canonical.encode()).hexdigest()}"

    def sign(key, message):
        return hmac.new(key, message.encode(), hashlib.sha256).digest()

    key = sign(("AWS4" + secret).encode(), date)
    for part in (region, "s3", "aws4_request"):
        key = sign(key, part)
    signature = hmac.new(key, to_sign.encode(), hashlib.sha256).hexdigest()
    request = urllib.request.Request(
        f"{endpoint}/{bucket}",
        method="PUT",
        headers={
            "x-amz-date": amz_date,
            "x-amz-content-sha256": payload_hash,
            "Authorization": (
                f"AWS4-HMAC-SHA256 Credential={access}/{scope}, "
                f"SignedHeaders=host;x-amz-content-sha256;x-amz-date, Signature={signature}"
            ),
        },
    )
    try:
        urllib.request.urlopen(request, timeout=10)
    except urllib.error.HTTPError as error:
        if error.code != 409:
            raise


def relay_environment(args, ports, relay_key, jwks_port):
    issuer = {
        "issuer": ISSUER,
        "command_audiences": [f"https://{RELAY_HOST}"],
        "token_class": "nip-fi+jwt",
        "algorithms": ["ES256"],
        "skew_seconds": 5,
        "maximum_assertion_age_seconds": 120,
        "jwks_uri": f"http://localhost:{jwks_port}/jwks.json",
        "jwks_transport": "mesh",
        "jwks_refresh_interval_seconds": 300,
        "jwks_hard_deadline_seconds": 3600,
        "maximum_command_age_seconds": 30,
        "authorized_principals": [OPERATOR_SUBJECT],
    }
    return {
        **os.environ,
        "DATABASE_URL": args.database_url,
        "REDIS_URL": args.redis_url,
        "RELAY_URL": f"wss://{RELAY_HOST}",
        "BUZZ_BIND_ADDR": f"127.0.0.1:{ports['relay']}",
        "BUZZ_HEALTH_PORT": str(ports["health"]),
        "BUZZ_METRICS_PORT": str(ports["metrics"]),
        "BUZZ_S3_ENDPOINT": args.s3_endpoint,
        "BUZZ_S3_ACCESS_KEY": args.s3_access_key,
        "BUZZ_S3_SECRET_KEY": args.s3_secret_key,
        "BUZZ_S3_BUCKET": args.s3_bucket,
        "BUZZ_RELAY_PRIVATE_KEY": relay_key,
        "BUZZ_REQUIRE_AUTH_TOKEN": "false",
        "BUZZ_RECONCILE_CHANNELS": "true",
        # The tombstone step needs the relay to advertise a push descriptor.
        # Nothing is delivered, so the gateway URL is never contacted.
        "BUZZ_PUSH_ENABLED": "true",
        "BUZZ_PUSH_GATEWAY_DELIVERY_URL": "http://127.0.0.1:9/v1/deliveries",
        "BUZZ_NIP_FI_MODE": "enforce",
        "BUZZ_NIP_FI_MAX_CONNECTION_LIFETIME_SECS": "3600",
        "BUZZ_NIP_FI_ISSUERS": json.dumps([issuer]),
        "BUZZ_NIP_FI_COMMUNITIES": json.dumps(
            [
                {"canonical_uri": f"https://{host}", "authorized_issuers": [ISSUER]}
                for host in (RELAY_HOST, SECOND_HOST)
            ]
        ),
    }


def wait_ready(relay, health_port, timeout=120):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if relay.poll() is not None:
            raise SystemExit(f"relay exited during start-up (code {relay.returncode})")
        try:
            with urllib.request.urlopen(
                f"http://127.0.0.1:{health_port}/_readiness", timeout=2
            ) as response:
                if response.status == 200:
                    return
        except (urllib.error.URLError, OSError):
            pass
        time.sleep(0.5)
    raise SystemExit("relay did not become ready")


WORKDIR_MARKER = ".nip-fi-live-workdir"


def prepare_workdir(workdir: Path):
    """Uses only a directory this check owns, and never loosens or tightens a
    directory it did not create.

    A new directory is created here and set to 0700. An existing one must be
    empty or carry the marker an earlier run left; anything else (`$HOME`,
    `/tmp`, a project directory) is refused untouched.
    """
    if workdir.exists():
        if not workdir.is_dir():
            sys.exit(f"refusing work directory {workdir}: it is not a directory")
        if any(workdir.iterdir()) and not (workdir / WORKDIR_MARKER).exists():
            sys.exit(
                f"refusing work directory {workdir}: it already exists and is not "
                "empty. Pass a new or empty directory (or one an earlier run of "
                "this check created)."
            )
    else:
        workdir.mkdir(parents=True)
        workdir.chmod(0o700)
    (workdir / WORKDIR_MARKER).touch()


def main():
    # A dedicated database on the docker-compose.yml local-dev server (same
    # throwaway user, not a secret). It is created on first use and reset on
    # every run; any other database name is refused.
    default_db = "postgres://buzz:buzz_dev@localhost:5471/buzz_nip_fi"  # sadscan:disable np.postgres.1
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--workdir", required=True, type=Path)
    parser.add_argument(
        "--relay-bin", default=str(REPO_ROOT / "target/debug/buzz-relay")
    )
    parser.add_argument(
        "--database-url", default=os.environ.get("NIP_FI_DATABASE_URL", default_db)
    )
    parser.add_argument(
        "--redis-url",
        default=os.environ.get("NIP_FI_REDIS_URL", "redis://localhost:6471"),
    )
    parser.add_argument(
        "--s3-endpoint",
        default=os.environ.get("NIP_FI_S3_ENDPOINT", "http://localhost:9471"),
    )
    parser.add_argument(
        "--s3-access-key", default=os.environ.get("NIP_FI_S3_ACCESS_KEY", "buzz_dev")
    )
    parser.add_argument(
        "--s3-secret-key",
        default=os.environ.get("NIP_FI_S3_SECRET_KEY", "buzz_dev_secret"),
    )
    parser.add_argument("--s3-bucket", default="buzz-media")
    parser.add_argument("--psql", default=os.environ.get("PSQL", "psql"))
    parser.add_argument("--assertion-lifetime-secs", type=int, default=20)
    args = parser.parse_args()

    workdir: Path = args.workdir.resolve()
    if workdir == REPO_ROOT or REPO_ROOT in workdir.parents:
        sys.exit(
            f"refusing work directory {workdir}: it is inside the repository, "
            "and it holds a test member key. Use a directory outside the repo."
        )
    # Fail on an unsafe database before anything is created or started.
    check_throwaway_database(args.database_url)
    prepare_workdir(workdir)
    for stale in ("fixture.json", "fixture-result.json", "markers.jsonl", "relay.log"):
        (workdir / stale).unlink(missing_ok=True)

    nsec, pubkey = new_member_key()
    relay_key = secrets.token_hex(32)
    issuer = Issuer(args.assertion_lifetime_secs)
    audiences = {f"https://{RELAY_HOST}", f"https://{SECOND_HOST}"}
    account_requests = []

    ensure_bucket(
        args.s3_endpoint, args.s3_access_key, args.s3_secret_key, args.s3_bucket
    )
    env = reset_database(args.database_url, args.psql)
    ids = seed(env, args.psql, pubkey)

    jwks_servers = serve_dual_loopback(jwks_handler(issuer))
    account_server = serve(account_handler(issuer, pubkey, audiences, account_requests))
    ports = {"relay": free_port(), "health": free_port(), "metrics": free_port()}
    ca_pem, context = make_tls(workdir)

    relay_log = workdir / "relay.log"
    with open(relay_log, "wb") as log:
        relay = subprocess.Popen(
            [args.relay_bin],
            env=relay_environment(args, ports, relay_key, jwks_servers[0].server_port),
            stdout=log,
            stderr=subprocess.STDOUT,
            cwd=REPO_ROOT,
        )
    stopping = threading.Event()
    signal.signal(signal.SIGTERM, lambda *_: stopping.set())
    signal.signal(signal.SIGINT, lambda *_: stopping.set())

    front = None
    try:
        wait_ready(relay, ports["health"])
        front = TlsFront(
            context,
            {
                RELAY_HOST: ports["relay"],
                SECOND_HOST: ports["relay"],
                ACCOUNT_HOST: account_server.server_port,
            },
        )
        (workdir / "fixture.json").write_text(
            json.dumps(
                {
                    "relay_origin": f"https://{RELAY_HOST}",
                    "second_origin": f"https://{SECOND_HOST}",
                    "account_origin": f"https://{ACCOUNT_HOST}",
                    "front_port": front.port,
                    "ca_pem": str(ca_pem),
                    "nsec": nsec,
                    "pubkey": pubkey,
                    "channel_id": ids["relay"]["channel"],
                    "second_channel_id": ids["second"]["channel"],
                    "community_id": ids["relay"]["community"],
                    "second_community_id": ids["second"]["community"],
                    "assertion_lifetime_secs": args.assertion_lifetime_secs,
                    "markers": str(workdir / "markers.jsonl"),
                    "relay_log": str(relay_log),
                },
                indent=2,
            )
        )
        (workdir / "fixture.json").chmod(0o600)
        print(f"fixture ready: {workdir / 'fixture.json'}", flush=True)
        while not stopping.is_set() and relay.poll() is None:
            stopping.wait(0.5)
    finally:
        alive = relay.poll() is None
        returncode = relay.returncode
        if alive:
            relay.terminate()
            try:
                relay.wait(timeout=15)
            except subprocess.TimeoutExpired:
                relay.kill()
                relay.wait()
        (workdir / "fixture-result.json").write_text(
            json.dumps(
                {
                    "relay_alive_at_stop": alive,
                    "relay_returncode": returncode,
                    "account_requests": account_requests,
                    "mints": list(issuer.mints),
                },
                indent=2,
            )
        )
        for jwks_server in jwks_servers:
            jwks_server.shutdown()
        account_server.shutdown()
        for secret_file in ("front-key.pem", "front-chain.pem"):
            (workdir / secret_file).unlink(missing_ok=True)
        # The run is over: the test member key is no longer needed on disk.
        # The oracle reads the rest of fixture.json afterwards.
        fixture_file = workdir / "fixture.json"
        if fixture_file.exists():
            kept = json.loads(fixture_file.read_text())
            kept["nsec"] = "removed at teardown"
            fixture_file.write_text(json.dumps(kept, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
