# Federated authorization deployment

Buzz ships its provider-neutral authorization runtime inactive. The stock
`buzz-relay` binary does not contain an identity-provider integration and does
not register the lifecycle operator API. This is deliberate: an OSS deployment
must supply its own verified identity input, provider policy, and operator
authentication instead of inheriting a permissive example.

This guide explains the integration required to deploy that runtime. It is for
relay deployers building a deployment-specific relay binary or composition
crate. It is not necessary for a normal local Buzz installation.

## Choose the integration model

Buzz has two related, but different, identity paths:

| Model | Appropriate when | Activation |
| --- | --- | --- |
| Relay-verified corporate identity | One deployment uses an asymmetric JWT issuer and the built-in binding policy | Configure the stock relay as described in [CORPORATE_IDENTITY.md](CORPORATE_IDENTITY.md) |
| Provider-neutral authorization | A deployment needs its own policy provider, exact per-community modes, bounded authorization leases, durable invalidation, and optional lifecycle operations | Build a deployment composition and follow this guide |

Do not turn on both paths independently for the same request and then choose the
more permissive answer. A provider-neutral deployment must have one explicit
source of verified evidence and one provider policy for each evaluating
community.

The client binding indicator is also separate. Authorization works without a
client indicator, and an indicator must never grant access. The stock desktop
client and relay do not gain a presentation surface merely because protected
authorization is installed.

## What happens on a protected request

For each request, the relay:

1. Resolves the community from trusted host state. The request cannot choose
   its authorization community or provider profile.
2. Verifies the Nostr proof and obtains verified federated evidence from the
   configured deployment adapter.
3. Calls the provider registered for that exact community and capability.
4. Joins the provider decision with authoritative binding and policy state.
5. Issues a bounded lease only in `enforce` mode.
6. Rechecks durable invalidation state while the lease is in use.
7. Denies the operation if any required component is absent, stale,
   unavailable, ambiguous, or inconsistent.

The same policy boundary covers WebSocket operations, the HTTP event/query
bridge, protected media, Git, audio, moderation, and invite operations. Public
health, readiness, and discovery routes retain their documented exemptions.

## Prerequisites

Before writing the deployment adapter:

- Run the database migrations from the exact relay revision being deployed.
- Create every protected community through the normal provisioning path. Each
  configured community UUID must exist in the database host map.
- Configure durable PostgreSQL, Redis, and object storage. Restore protection
  uses object storage as an independent witness for PostgreSQL high-water
  state.
- Configure a stable relay signing key and production TLS.
- Choose either direct assertion delivery or a trusted-proxy design. A trusted
  proxy must strip every inbound copy of the assertion header, inject exactly
  one verified value, and prevent clients from reaching the relay directly.
- Define the provider's authoritative policy source, outage behavior, and
  maximum acceptable invalidation delay.
- Put all pseudonymization keys and provider credentials in a secret manager.
  Do not place them in source control or logs.

## Build the deployment composition

The deployment composition replaces two stock startup choices: the empty
provider registry and, if lifecycle operations are required, the absence of an
operator router.

### 1. Implement an authorization provider

Implement `buzz_auth::AuthorizationProvider` for the deployment's policy
adapter. The implementation must:

- return a profile ID fixed by trusted server configuration;
- evaluate only the exact typed request it receives;
- perform no binding, membership, or lifecycle mutation;
- return an explicit allow, deny, or unavailable decision;
- use bounded, cancellation-safe asynchronous I/O; and
- treat malformed, conflicting, stale, or incomplete upstream state as deny or
  unavailable, never allow.

Provider cache updates may be used, but each update must become visible
atomically. Dropping a timed-out provider future must not leave a partial
policy mutation behind.

The OSS repository intentionally contains no production provider. Test
providers are examples of the trait shape only and must not be used in a real
deployment.

### 2. Supply verified identity evidence

There are two supported composition shapes:

- Use the built-in asymmetric JWT verifier and its trusted assertion
  provenance. Configure it according to [CORPORATE_IDENTITY.md](CORPORATE_IDENTITY.md),
  then install the provider-neutral runtime without a separate evidence
  resolver.
- Install a deployment-owned `VerifiedProviderEvidenceResolver`. The resolver
  may expose only evidence that was already verified and bound to trusted
  request or connection state. It must return `Ambiguous` for multiple or
  conflicting sources.

The resolver is not a shortcut for parsing an arbitrary header. Raw tokens,
untrusted transport classifications, and client-selected domains cannot
construct verified provider evidence.

### 3. Register providers by exact community

Construct one `ProductionProviderRegistry` entry for every community using
`shadow`, `verify_only`, or `enforce`. Duplicate entries and missing providers
stop startup.

The essential installation boundary looks like this:

```rust
use std::sync::Arc;

use buzz_auth::AuthorizationProvider;
use buzz_core::CommunityId;
use buzz_relay::authorization_runtime::production::{
    install_from_environment_with_providers_and_evidence,
    ProductionProviderRegistry,
};
use buzz_relay::authorization_runtime::transport::VerifiedProviderEvidenceResolver;

async fn install_deployment_authorization(
    state: &Arc<buzz_relay::AppState>,
    community: CommunityId,
    provider: Arc<dyn AuthorizationProvider>,
    evidence: Option<Arc<dyn VerifiedProviderEvidenceResolver>>,
) -> anyhow::Result<()> {
    let providers = ProductionProviderRegistry::new([(community, provider)])?;

    install_from_environment_with_providers_and_evidence(
        state,
        providers,
        evidence,
    )
    .await?;

    Ok(())
}
```

For multiple communities, add one exact `(CommunityId, provider)` entry for
each evaluating community. Do not install a global fallback provider.

Call this function after constructing `AppState` and before building or
serving the Axum router. Installation initializes restore and invalidation
state before protected transports become reachable. A partial installation is
a startup error.

The stock `main.rs` calls `install_from_environment` with an empty registry.
Setting a non-`off` evaluating mode on the stock binary therefore fails startup
instead of activating an incomplete deployment.

## Configure community modes

`BUZZ_PROTECTED_AUTHORIZATION_DOMAINS` is a comma-separated list of
`<community-uuid>:<mode>` entries:

```dotenv
BUZZ_PROTECTED_AUTHORIZATION_DOMAINS=\
11111111-1111-4111-8111-111111111111:shadow,\
22222222-2222-4222-8222-222222222222:deny_protected
```

| Mode | Provider evaluated | Protected access behavior |
| --- | --- | --- |
| `off` | No | Legacy behavior; use only before the community has been durably activated |
| `shadow` | Yes | Observe provider decisions without granting authority or protecting surfaces |
| `verify_only` | Yes | Produce a bounded display-only verification result; it grants no access |
| `enforce` | Yes | Require a successful final decision and issue a bounded access lease |
| `deny_protected` | No | Keep protected surfaces active while denying all protected access |

An absent or blank domain list leaves the provider-neutral runtime inactive.
After a community has been activated in `enforce` or `deny_protected`, startup
rejects removing it or changing it to `off`, `shadow`, or `verify_only`. This
prevents a stale configuration or rollback from silently reopening protected
surfaces. Use `deny_protected` as the emergency fail-closed state.

Additional runtime configuration:

| Variable | Required | Meaning |
| --- | --- | --- |
| `BUZZ_PROTECTED_AUTHORIZATION_PROFILE` | No | Trusted provider profile ID; defaults to `current-membership-v1` |
| `BUZZ_PROTECTED_AUTHORIZATION_LEASE_SECONDS` | No | Positive maximum lease duration; defaults to 300 seconds |
| `BUZZ_AUTHORIZATION_AUDIT_PSEUDONYM_KEY_HEX` | For evaluating modes | Dedicated 32-byte hex key for audit-only pseudonymous evidence |
| `BUZZ_AUTHORIZATION_AUDIT_PSEUDONYM_KEY_EPOCH` | For evaluating modes | Positive integer identifying the pseudonymization-key epoch |
| `BUZZ_PROTECTED_AUTHORIZATION_RESTORE_BOOTSTRAPS` | For protected modes | Exact comma-separated `<community-uuid>=<bootstrap-uuid>` mappings |

For example:

```dotenv
BUZZ_PROTECTED_AUTHORIZATION_PROFILE=current-membership-v1
BUZZ_PROTECTED_AUTHORIZATION_LEASE_SECONDS=300
BUZZ_AUTHORIZATION_AUDIT_PSEUDONYM_KEY_HEX=<64 lowercase hex characters>
BUZZ_AUTHORIZATION_AUDIT_PSEUDONYM_KEY_EPOCH=1
BUZZ_PROTECTED_AUTHORIZATION_RESTORE_BOOTSTRAPS=\
22222222-2222-4222-8222-222222222222=<immutable non-nil UUID>
```

Generate each pseudonymization key independently. Do not reuse a JWT signing
key, relay key, operator key, or client-status privacy key.

## Provision restore protection

Before a community first enters `enforce` or `deny_protected`, provision its
object-store checkpoint exactly once. Choose a new non-nil bootstrap UUID,
record it in durable deployment configuration, and call:

```rust
use buzz_relay::authorization_runtime::restore::RestoreProtectionRuntime;

RestoreProtectionRuntime::provision_domain(
    &state.db,
    &state.git_store,
    community,
    bootstrap_id,
)
.await?;
```

Provisioning must be an explicit administrative step, not ordinary startup.
It uses a create-only object-store write and fails if the community was already
provisioned. Never generate a new bootstrap UUID on restart.

At subsequent startups, the configured UUID must match the checkpoint and the
database version vector must be at least as current as the witnessed floor.
Missing checkpoints, stale restores, unwitnessed authority advances, and
ambiguous interrupted commits all stop protected startup.

Back up both the authoritative database state and the independent checkpoint
store. Restoring only one side is not a supported recovery procedure.

## Stage the rollout

Use a separate deployment and evidence record for each stage:

1. **Disabled:** deploy the custom binary with no configured protected domains.
   Confirm ordinary relay behavior is unchanged.
2. **Shadow:** register the provider and observe bounded decision categories.
   Compare results with the authoritative policy source without changing
   access.
3. **Verify only:** validate exact identity-to-key matching and expiry behavior.
   Treat any client-visible status as presentation only.
4. **Provision:** create the independent restore checkpoint and record its
   immutable bootstrap UUID.
5. **Enforce:** activate one canary community, validate every protected surface,
   then expand only after the canary evidence passes.
6. **Advertise:** publish NIP-FI discovery only after complete, same-revision
   conformance and deployment checks succeed.

Do not use successful `shadow` observations as evidence that enforcement or
restore behavior works. Do not publish discovery merely because one process is
running in `enforce` mode.

## Install lifecycle operator routes when needed

Provider-neutral authorization does not require exposing lifecycle HTTP
routes. If a deployment needs list, preview, revoke, or rotate operations, it
must separately provide:

- an `OperatorAuthenticator` that returns short-lived, intent-bound grants;
- a `DurableOperatorExecutor`, normally `PostgresOperatorExecutor`, configured
  with dedicated operator-reference and audit pseudonymization keys; and
- an `OperatorClock` from trusted deployment time.

Then construct and merge the router explicitly:

```rust
use std::sync::Arc;

use buzz_relay::api::operator::lifecycle_router;
use buzz_relay::operator_runtime::OperatorRuntime;
use buzz_relay::router::build_router;

let operator_runtime = Arc::new(OperatorRuntime::new(
    operator_authenticator,
    durable_operator_executor,
    operator_clock,
));

let app = build_router(Arc::clone(&state))
    .merge(lifecycle_router(operator_runtime));
```

The stock router does not register these endpoints:

- `POST /operator/v1/lifecycle/list`
- `POST /operator/v1/lifecycle/preview`
- `POST /operator/v1/lifecycle/revoke`
- `POST /operator/v1/lifecycle/rotate`

The authenticator must bind its grant to the exact domain, operation UUID,
intent fingerprint, capability, expiry, actor, credential provenance, and any
independent approvals. Mutations are idempotent by operation UUID and intent;
reusing an operation UUID for different intent must be rejected. Do not expose
these routes until ingress authentication, rate limits, no-store response
handling, and audit retention have been reviewed for the deployment.

## Discovery and client presentation

NIP-11 discovery is a claim about a complete deployment, not an enablement
flag. A deployment must construct `ConformanceReadyNipFiDiscovery` from a
complete-stack conformance source and install it into `AppState`. There is no
environment-variable shortcut.

If trusted-proxy transport is advertised, the conformance source must include
origin-isolation evidence plus negative tests showing that direct relay access
and client-supplied header copies cannot bypass the proxy.

Relay-authenticated client status has an additional typed approval and
dedicated transport gate. Keep it inactive until its deployment, privacy, and
client compatibility checks pass. A missing, withdrawn, expired, or invalid
status must display as no indicator and must never affect authorization.

See [NIP_FI_RUNTIME_OPERATIONS.md](NIP_FI_RUNTIME_OPERATIONS.md) for session,
upgrade, rollback, restore, and privacy behavior, and
[nips/NIP-FI-RUNTIME-CONFORMANCE.md](nips/NIP-FI-RUNTIME-CONFORMANCE.md) for the
runtime evidence matrix.

## Pre-enforcement checklist

Before changing any community to `enforce`, verify all of the following on the
exact candidate revision:

```shell
cargo test -p buzz-auth
cargo test -p buzz-relay authorization_runtime
cargo test -p buzz-relay --test nip_fi_runtime_conformance
```

- Database migrations and startup reconciliation completed successfully.
- The host resolves to the intended community and cannot be selected by a
  client-controlled field.
- Exactly one provider is registered for the community and its profile matches
  server configuration.
- Verified evidence is bound to the Nostr signer or explicitly validated
  delegated owner as intended.
- Missing, expired, future, malformed, duplicated, and conflicting assertions
  are denied.
- Provider timeout, outage, cancellation, and stale-policy cases fail closed.
- WebSocket, HTTP bridge, media `GET`/`HEAD`/upload, Git read/write, audio,
  moderation, and invite paths have the expected decisions.
- Disconnect, logout, revocation, rotation, and dependency invalidation end
  affected sessions within the documented bound.
- A rotate-back attempt cannot reactivate a retired key pair.
- The restore checkpoint exists, matches its immutable bootstrap UUID, and
  rejects a staged stale-database restore.
- Trusted-proxy deployments pass direct-bypass and inbound-header-copy
  negative tests.
- Logs, metrics, traces, fixtures, and alerts contain no raw assertions,
  issuer-qualified subjects, display names, emails, or provider-private data.
- Discovery remains absent until every applicable conformance row passes at
  the same revision.

## Failure and rollback behavior

Treat startup refusal as a security control. Do not bypass failures for missing
providers, evidence provenance, verifiers, community host mappings, audit
keys, restore checkpoints, or worker initialization.

For an incident:

1. Remove NIP-FI discovery before or with the first rollback.
2. Change affected activated communities to `deny_protected` if access must be
   stopped.
3. Preserve binding, lifecycle, invalidation, audit, and restore-witness state.
4. Roll back only to a revision that understands the durable protected state;
   an activated community cannot safely return to legacy behavior by deleting
   configuration.
5. Re-run same-revision conformance before restoring discovery or client
   presentation.

Detailed restart and restore requirements are in
[NIP_FI_RUNTIME_OPERATIONS.md](NIP_FI_RUNTIME_OPERATIONS.md).
