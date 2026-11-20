# Mobile NIP-FI live check

Runs the mobile app's real hosted-relay sign-in, connect, query, upload and
media code against a local `buzz-relay` started with `BUZZ_NIP_FI_MODE=enforce`,
then reads the relay's own access log to decide pass or fail. It exists so a
mobile change that stops sending the NIP-FI assertion on some route is caught
here, not by a hosted community refusing the app.

Run it (not part of `just ci` or the normal `flutter test`):

```sh
. bin/activate-hermit
cargo build -p buzz-relay
mobile/scripts/nip_fi_live/run.sh [workdir]
```

It needs Postgres, Redis and S3-compatible storage at the addresses in
`fixture.py` (the defaults match `scripts/start-isolated-test-relay.sh`; override
with `NIP_FI_DATABASE_URL`, `NIP_FI_REDIS_URL`, `NIP_FI_S3_ENDPOINT`), plus `uv`
and `psql`. It resets the database it is pointed at.

## Pieces

- `fixture.py` starts the relay, a local test issuer (ephemeral ES256 key, JWKS
  over plain-HTTP loopback with `jwks_transport: "mesh"`), a fake account API,
  and a TLS front that routes by `Host` using a throwaway CA. All keys and the
  CA are generated at run time and never written outside the work directory.
- `test/nip_fi_live/` holds the journey, tagged `nip-fi-live` so a plain
  `flutter test` skips it. The tests redirect the hosted names
  (`*.nipfi.localhost`) to the TLS front with an in-process `HttpOverrides`,
  so the production hosted gate (https, no port, DNS domain) is unchanged and
  nothing binds :443 or touches the system trust store.
- `oracle.py` fails the run on a `method=nip_fi` denial outside the negative
  control, on a required route class that never succeeded (WebSocket auth,
  `/query`, `/upload`, `/media/*`, a second community's WebSocket auth), and on
  a missing log or a crashed relay.

Not covered: the iOS simulator and the notification extension (Layer B).
