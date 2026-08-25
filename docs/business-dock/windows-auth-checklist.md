# Windows V3.2 Auth Validation Checklist

> V3.2 status: NOT TESTED — RELEASE BLOCKED. Do not claim production support
> until the Gateway-backed flow below passes on a clean Windows device.

2026-08-21 environment audit: the available host is macOS 26.5.2 arm64 and has
no Windows runner, WebView2 runtime, Parallels, VMware, VirtualBox or QEMU Windows
guest. Docker/buildx exposes Linux targets only. These facts explain why the row
is still blocked; they are not Windows acceptance evidence.

- Verify Buzz pubkey/device challenge signing and revoked-device state.
- Verify 30-second Embed Session and one-time replay rejection.
- Inspect WebView2 cookie: HttpOnly, Secure, SameSite=None, Partitioned, Path=/,
  host-only, and unavailable to JavaScript.
- Verify `/api/session` rotates an in-memory-only CSRF token; wrong Origin,
  missing CSRF and wrong CSRF each fail with 403 before business handling.
- Verify Business-only, Workbench-only, global, and expiry recovery.

Record the Windows version, WebView2 runtime version, Tauri build identifier and Authentik version. Every row remains `NOT TESTED` until exercised on Windows; do not copy macOS results.

## Setup

- Build with strict `buzz://auth/callback`, `buzz://auth/logout-callback` and `buzz://auth/business-bootstrap` registration.
- Use organization-trusted HTTPS. Do not use the macOS local-CA loopback workaround as production evidence.
- Start with a clean WebView2 profile and a clean system-browser Authentik session.
- Capture only sanitized status/header evidence. Never capture codes, tokens, cookies, secrets, state, nonce or PKCE values.

## Workbench

- Workbench opens the system browser, not an embedded Authentik page.
- Authorization Code + S256 completes and the exact deep link returns to the same app instance.
- Duplicate callback delivery is ignored.
- Cancel, invalid state and expired transaction fail closed.
- App restart requires a Workbench re-entry but reuses the Authentik browser session without another password.

## Business Embed Session

- Opening Business Dock produces one `AUTH_REQUIRED`, with no retry loop.
- Explicit user action opens Business authentication in the system browser.
- Existing Authentik SSO avoids a second password prompt.
- `business-bootstrap` deep link is routed only to Business Dock.
- WebView2 redeems once, reaches the exact bound target and receives an HttpOnly Secure Business cookie.
- Replay, expiry, revocation, wrong audience, external target and malformed code all fail closed.
- One automatic recovery attempt occurs; further attempts require explicit user action.

## Isolation and logout

- Workbench logout does not silently become Business logout.
- Business logout clears only the Business WebView session.
- Authentik/global logout behavior is recorded separately.
- App close/reopen and machine restart behavior are recorded separately for Workbench, Authentik and Business.
- Bridge traffic contains only allowed V3 auth status fields and never credentials or groups.

## Decision

Mark Windows `PASS` only if the exact packaged WebView2 path completes. If native system-browser cookie reuse unexpectedly works, retain Embed Bootstrap unless the simpler path also passes security review and repeatability testing across supported WebView2 versions.
