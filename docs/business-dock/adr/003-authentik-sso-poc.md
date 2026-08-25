# ADR 003: Authentik SSO POC

- Status: Superseded for desktop by ADR 004; retained for Web decision history
- Date: 2026-08-20

## Context

Workbench and Business Dock need one organizational sign-in experience without turning Workbench into a token broker. Business is cross-origin and embedded. Buzz Desktop also separates its Tauri WebView cookie jar from the user's system browser.

Authentik supports Authorization Code, PKCE, strict redirect URI allowlists, public clients, confidential clients, and RP-initiated logout. Its current official Compose topology uses PostgreSQL, server, and worker; Redis was removed in Authentik 2025.10.

## Options

- A. Two independent clients, with Business OIDC promoted to a top-level/system-browser context.
- B. Direct Workbench-to-Business token handoff.
- C. A one-time Business Embed Session broker.
- Direct iframe OIDC was also measured as a transport variant of A, not treated as a separate trust model.

## Observed results

Authentik 2026.8.0's default authentication flow returned `X-Frame-Options: DENY`; a real Playwright Chromium iframe navigated to `chrome-error://chromewebdata/`. The official Compose PostgreSQL/server/worker topology reached healthy and its ready endpoint returned 200. Bridge V3 and its expiry/recovery fixture passed automated tests. Real two-client OIDC, trusted TLS, macOS WebKit cookie reuse, Windows WebView2 cookie reuse, second-prompt behavior, and logout propagation remain `NOT TESTED`.

## Decision

Choose architecture A for the primary path:

1. Workbench and Business are separate Authentik OIDC clients.
2. Workbench Web uses Authorization Code + PKCE in a top-level browser context.
3. Workbench Desktop uses the system browser and the existing `buzz://` deep-link callback.
4. Business exchanges its own code server-side and owns an opaque HttpOnly Secure session cookie.
5. Bridge V3 carries only authentication state and the minimal `{ subject, displayName }` summary. It never carries access tokens, refresh tokens, authorization codes, cookies, groups, or full identity claims.
6. Authentik is not added to Business Dock's `frame-src`. Interactive authentication is not performed inside the Dock iframe.

Architecture B, direct Workbench-to-Business token handoff, is rejected because it couples audiences and lifetimes, expands the postMessage attack surface, and makes Workbench a credential broker.

Architecture C, a one-time Embed Session broker, is `FALLBACK ONLY`. It is justified only if real macOS or Windows tests prove that a Business session established in the system browser cannot be made available safely to the embedded WebView. Any fallback must use a single-use, audience-bound, short-lived code redeemed by Business; it must never hand an Authentik token to the iframe.

## Fallback strategy

Do not build the broker during this POC. First run the documented Web, WebKit, and WebView2 matrix. If either desktop WebView cannot consume a Business session safely after top-level authorization, design the one-time exchange as a separate reviewed change with replay protection, strict audience and redirect binding, short expiry, audit, and revocation.

## Iframe decision

Do not navigate the embedded Business iframe through Authentik. The existing sandbox intentionally lacks top-navigation permission, the host CSP allows only the Business Origin in `frame-src`, and browser/WebView third-party-cookie behavior is not a stable foundation for interactive OIDC. The Dock sends a single non-interactive `CHECK_AUTH`; after `AUTH_REQUIRED`, the user chooses the explicit top-level/system-browser path. There is no retry loop.

## Storage and validation

`oidc-client-ts` performs code + PKCE and validates the OIDC response. Workbench keeps the User (and therefore tokens) in an in-memory store. Only transient state/nonce/PKCE correlation data is placed in sessionStorage so a browser redirect or desktop callback can complete. No refresh token is written to localStorage.

The Business Auth Mock uses `openid-client`, validates PKCE/state/nonce, exchanges codes only on the server, and puts an opaque ID in a host-only HttpOnly cookie. Its in-memory session store is POC-only and is not a production session implementation.

## Consequences

Web can plausibly achieve near-seamless second-client login by reusing the top-level Authentik session and then returning to a same-site Business iframe. Desktop remains conditional because the system browser and WebView do not normally share cookies. Production V3 must not claim desktop seamless SSO until both WebKit and WebView2 tests pass or the fallback broker is designed and threat-modeled.

## References

- [Authentik OAuth2/OIDC provider](https://docs.goauthentik.io/add-secure-apps/providers/oauth2/)
- [Authentik Docker Compose installation](https://docs.goauthentik.io/install-config/install/docker-compose/)
- [Authentik front-channel and back-channel logout](https://docs.goauthentik.io/add-secure-apps/providers/oauth2/frontchannel_and_backchannel_logout/)
- [Tauri v2 deep linking](https://v2.tauri.app/plugin/deep-linking/)
- [oidc-client-ts](https://github.com/authts/oidc-client-ts)
