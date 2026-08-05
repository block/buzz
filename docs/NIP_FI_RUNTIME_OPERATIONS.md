# NIP-FI runtime operations

This runbook covers provider-neutral NIP-FI session/discovery behavior and the
separate disabled relay-authenticated client-status contract. It does not
authorize enabling a provider, a client presentation surface, or a conformance
claim.

For the deployment composition, provider registration, configuration, staged
activation, and pre-enforcement checks, see
[FEDERATED_AUTHORIZATION_DEPLOYMENT.md](FEDERATED_AUTHORIZATION_DEPLOYMENT.md).

## Session and reconnect behavior

For WebSocket authorization, the assertion belongs on the upgrade request and
fresh NIP-42 proof follows on that connection. A direct lease ends at the
earliest assertion, binding, policy, or implementation bound. Base V1 has no
in-connection assertion renewal: expiry requires a new connection, a fresh
upgrade assertion, and fresh NIP-42 proof.

Delegated sessions require a separately validated delegation, an active owner
binding, and a positive finite configured implementation maximum. A cached
owner lease is not substitute authority. Reconnect requires fresh delegate
proof and revalidation of every dependency.

When an observed binding, identity, key, policy, or delegation dependency
becomes invalid, reject protected operations or close the affected connection
within the documented detection bound. A polling deployment must publish its
maximum detection latency and must not claim immediate revocation.

The optional assertion `iat` check uses the shared injected authorization
clock. The current JWT library still evaluates `exp` and `nbf` with its own
system-clock source, so operators must maintain host clock synchronization and
must not claim fully centralized assertion time until that library boundary is
made injectable.

`iat` is optional in Base V1. When present, a malformed or more-than-60-second
future value is rejected. A `kid` absent from a still-fresh JWKS set is denied
without an immediate refetch; the default set lifetime is 300 seconds. Issuers
must overlap old and new signing keys for at least the cache lifetime plus the
documented clock allowance. Refresh after expiry is single-flight, and refresh
or issuer failure never falls back to an unverified key.

The assertion header is singular. Multiple field lines, a comma-combined
value, invalid UTF-8, or an empty value is denied before verification. A
trusted-proxy adapter must prove origin isolation and that it stripped every
inbound copy before injecting exactly one assertion; merely observing the
configured header is not transport provenance.

Protected media downloads include both `GET` and `HEAD`. An enforcing domain
cannot expose either method without current authority; a deployment that wants
public media needs a separately reviewed public-media policy rather than an
implicit read bypass.

Client status is presentation-only. It expires independently of an
authorization lease and is cleared on expiry, disconnect, relay-key change,
domain change, or author change. A status cannot renew a session, authorize an
operation, create a binding, mint a lease, or mutate membership.

## Upgrade sequence

1. Upgrade and reconcile durable authorization, binding, lifecycle, lease, and
   status-revision-floor state before enabling any behavior.
2. Deploy servers with NIP-FI discovery absent and the client-status
   presentation gate disabled.
3. Run every applicable NIP-FI row against the exact candidate revision. For
   `trusted-proxy`, attach enforced origin-isolation evidence plus negative
   direct-bypass and inbound-header-copy tests.
4. Confirm mixed-version servers all omit discovery. Never advertise based on
   a per-process flag or a partial fleet.
5. Supply a complete-stack conformance input only after the whole serving fleet
   runs the reviewed revision and all applicable rows pass.
6. Supply the typed client-presentation approval only after its deployment,
   privacy, and client-compatibility gates pass at one exact revision. The
   stock binary has no environment or boolean shortcut; without that injected
   proof it cannot construct the presentation permit or install the dedicated
   exact-connection transport.

Old clients ignore unknown status events, and old servers emit none. New
clients must default to no indicator when status is absent, invalid, expired,
withheld, or unsupported. NIP-FI authorization behavior must remain identical
whether client presentation code is present or absent.

## Rollback

Remove the complete-stack readiness input before or with the first server
rollback so NIP-11 immediately omits NIP-FI discovery. Do not leave discovery
enabled for a mixed or unreviewed fleet.

Client-status rollback requires no authority migration: the events are
ephemeral and display-only. Disconnect affected clients or wait no longer than
the bounded status lifetime; clients clear on either condition. Never translate
a cached status into an authorization decision during rollback.

Preserve durable authorization and lifecycle state. Preserve and reconcile the
status revision floor so a restored older process cannot emit a lower revision
that a client might mistake for current state. If that state is unavailable,
emit no status.

## Public projection retirement

The privacy-approved NIP-85 label projection is optional and never authority.
After an authoritative revoke or rotate commits, the lifecycle integration must
derive internal retirement work from the committed lifecycle record. The work
contains only the server-resolved domain, old public Nostr key, relay author,
operation identifier, and opaque binding generation. It must not contain
issuer, subject, display label, provider claims, actor, or free-text reason.
The reconciler idempotently replaces an active projection with the existing
inactive, label-free parameterized event.

A read, clock, build, or write failure must not roll back the already committed
lifecycle mutation. Retry the same domain/key request. If a write committed but
its acknowledgement was lost, the retry observes the inactive replacement and
terminates without another write.

The runtime materializes committed revoke/rotate operations into an internal
durable queue, fences active publication and retirement with the exact binding
generation, and drains unfinished projection and delivery work before protected
runtime installation and after restart. Periodic discovery is the crash-window
backstop. The active projection TTL remains defense in depth. Authenticated
lifecycle routes and durable operator audit remain separately owned.

## Backup and restore

Back up authoritative binding/lifecycle state, policy state, cryptographic
secrets required by deployment policy, and durable status revision/floor state
using the owning subsystem's procedure. Protect the dedicated client-status
privacy key as a secret and never reuse it across unrelated deployments.

Do not back up or restore:

- authorization or provider caches;
- direct or delegated leases;
- WebSocket connection state;
- client presentation caches;
- emitted client-status events; or
- ordinary event/pubsub copies of client status, because none may exist.

After restore, start with discovery absent and presentation disabled. Rebuild
authorization decisions from authoritative state, reconcile revision floors,
reconcile committed projection retirements, and reconnect clients with fresh
assertions/proofs. If the relay signing key or client-status privacy key
changed, treat every old presentation as invalid. A restored service must
complete same-revision conformance again before discovery can return.

## Privacy and observability

Logs, metrics, traces, fixtures, and NIP-11 output must not contain raw bearer
assertions or unredacted issuer, subject, audience, tenant URL, claim name,
display name, email, or provider-private metadata. Use bounded categorical
failure classes and pseudonymous correlation where necessary.

Alert on aggregate validation failures, revision-source unavailability,
dedicated-transport unavailability after a future gate is approved, and
dependency-invalidation lag. Do not include the rejected private value in an
alert. The presence or absence of a client status is not evidence of access and
must never drive an authorization SLO.
