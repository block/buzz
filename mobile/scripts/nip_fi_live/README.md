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
`fixture.py` (the ports match `scripts/start-isolated-test-relay.sh`; override
with `NIP_FI_DATABASE_URL`, `NIP_FI_REDIS_URL`, `NIP_FI_S3_ENDPOINT`), plus `uv`
and `psql`.

It drops and recreates the `public` schema of its database on every run, so it
refuses any database whose name does not contain `nip_fi` or that is not on this
machine. The default is `buzz_nip_fi` on the local-dev Postgres, created on first
use. The work directory holds a test member key while the run is live, so it must
be outside the repository; the key is removed from `fixture.json` at teardown.

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
  control, on a missing step marker, and on a missing log or a crashed relay. It
  binds each required route class to the step it belongs to (a success in
  another step does not count): WebSocket auth in `connect`, `/query` in `query`
  and `query_after_expiry`, `/upload` in `upload`, `/media/*` in `media`,
  `native_media` and `native_media_after_expiry`, and the second community's
  WebSocket auth in `tombstone`. It also checks the expiry path was real: no
  session or query traffic during the wait, the cached assertion had expired,
  and a newer assertion was minted for the read after it.

Not covered: the iOS simulator and the notification extension (Layer B).
