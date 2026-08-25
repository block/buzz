# Logout semantics

```mermaid
flowchart TD
  B[Business-only] --> B1[Revoke current BusinessSession]
  B --> B2[Clear Business and CSRF cookies]
  B --> B3[Revoke pending Embed sessions]
  W[Workbench-only] --> W1[Revoke Workbench and linked sessions]
  W --> W2[Clear Workbench in-memory identity]
  W --> W3[Keep Authentik top-level SSO]
  G[Global] --> G1[Workbench revocation]
  G --> G2[Authentik RP-initiated logout]
  G --> G3[Logout callback]
```

Business-only preserves Workbench OIDC, Authentik SSO, Buzz identity and channel
state. Workbench-only does not claim to exit all systems and may reuse top-level
SSO on the next login. Global additionally invokes the provider end-session
flow.

The back-channel endpoint validates logout-token signature, issuer, Business
audience/client, expiry, events claim, and the OIDC session subject. It revokes
by `sid` when present and falls back to the issuer-scoped `sub` when the provider
omits `sid`. Deployment must register and exercise that URI before marking it
PASS.

The Business Dock toolbar exposes all three user-facing scopes with distinct
labels. Workbench-only and Global logout also notify the embedded Business Host
and clear the Dock authentication state; only Global starts provider logout.
