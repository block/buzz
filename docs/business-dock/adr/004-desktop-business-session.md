# ADR 004: Desktop Business Session Bootstrap

> V3.2 update: ADR 005 supersedes the POC system-browser Business authorization
> flow for production Desktop. Embed issuance is now an authenticated
> Workbench-to-Gateway API call. This ADR remains V3.1 POC history.

- Status: Accepted for V3 macOS; Windows candidate pending validation
- Date: 2026-08-20
- Supersedes: ADR 003's `FALLBACK ONLY` position for desktop

## Context

Workbench and Business are separate Authentik clients and must retain separate tokens and application sessions. Web can reuse the Authentik top-level SSO session. Desktop authorizes in the system browser but renders Business in Tauri's WebView, which is a separate session context. The decision is whether to depend on native cookie reuse or bootstrap a new Business-owned WebView session.

## Observed Web Results

Real two-client SSO passed. Workbench public code+PKCE login, Business confidential-client authorization, no-second-password behavior, Business HttpOnly cookie, Dock authentication, refresh, close/reopen, Business-only logout and passwordless recovery were exercised. Both clients received the expected `poc-user` groups claim. Authentik itself remained outside the iframe.

## Observed macOS Results

The packaged Debug app opened Authentik in the system browser, returned through the strict `buzz://auth/callback`, completed PKCE token exchange and authenticated Workbench. After app restart the in-memory Workbench session was absent, while the system-browser Authentik session persisted and completed the next authorization without credentials.

The final Business Dock UI check was blocked by an unrelated upstream Buzz first-run relay error. Real Business OIDC plus one-time redemption was nevertheless exercised against Authentik in an isolated second context. No supported mechanism established that a Business cookie written to the system browser would become a WKWebView cookie.

## Observed Windows Results

`NOT TESTED`. No Windows/WebView2 runner was available. The required validation is recorded in `windows-auth-checklist.md`.

## Option A: Native Authentik SSO

Business completes OIDC in the system browser and the embedded WebView somehow reuses the Business cookie. This is attractive only if the platform provides a standard, repeatable cookie path. No such path was established for macOS, and the design must not rely on cookie injection, private APIs or weakened cookie settings.

## Option B: Authentik + Embed Session

Business authenticates its own system-browser request, then issues a high-entropy one-time code. Business Dock sends that code only to Business `/embed/bootstrap`; Business atomically consumes it and writes a new independent HttpOnly WebView session before redirecting to the bound target.

## Security Comparison

Option A has less application code but would hide a cross-context cookie dependency. Option B makes the transport explicit: 256-bit random code, SHA-256 hash at rest, 30-second TTL, exact `business-dock` audience, allowlisted target, authenticated-session binding, atomic single use and revocation. Invalid, expired, used and revoked codes fail closed. Neither option permits Authentik tokens, credentials, cookies, client secrets or full claims in the Bridge or deep link.

## Operational Comparison

Option A would be operationally simpler only if WebKit and WebView2 shared the required session reliably. Option B adds durable ticket storage, migration/cleanup, monitoring, audit and revocation, but behaves consistently without platform cookie hacks. The POC uses SQLite and in-memory Business sessions; production must use the normal Business session service and durable migration tracking.

## Decision

- Web: choose Option A, native two-client Authentik SSO.
- macOS: choose Option B, Authentik identity plus One-Time Embed Session Bootstrap.
- Windows: pending validation; Option B is the default candidate.

The macOS choice is architectural, not a claim that the final native Dock UI
row passed. V3.2 removed the Relay first-run blocker with the isolated clean
acceptance harness and superseded the POC authority in ADR 005. The packaged
WKWebView row remains governed by the current V3.2 verification matrix.

## Platform Strategy

Keep the identity proof identical across platforms: two clients, code flow, PKCE where applicable, strict redirects and Business-owned authorization. Vary only the last session transport: native Business session on Web, one-time Business bootstrap on Desktop.

## Fallback

If a bootstrap fails and Workbench is still authenticated, permit one automatic reissue and restore the last resolved BusinessResource. If Workbench is not authenticated, return to Authentik. A second failure requires explicit user retry. Never fall back to token handoff or cookie copying.

## POC Transport Exception

The developer machine trusted its local CA in the login keychain but WKWebView did not trust that chain for OIDC fetches. The POC uses a narrow native loopback command for exact Workbench token/userinfo/JWKS paths. Browser authorization remains HTTPS. This is a certificate-harness exception, not the production architecture; production must use directly trusted HTTPS.

## Consequences

Desktop adds one Business round trip and a small revocable ticket lifecycle. Workbench remains a launcher rather than a credential broker. Authentik is never framed, CSP remains narrow, Business owns its cookie, and V3.1 does not introduce RBAC, device management, MCP or ERP behavior.

## References

- [Authentik OAuth2/OIDC provider](https://docs.goauthentik.io/add-secure-apps/providers/oauth2/)
- [Tauri v2 deep linking](https://v2.tauri.app/plugin/deep-linking/)
