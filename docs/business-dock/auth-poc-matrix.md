# Business Dock V3.2 Verification Matrix

Recorded through 2026-08-21. `PASS` means the named path was actually exercised.
`PARTIAL` means only part of the named platform path has direct evidence.
Static fixtures never upgrade a real OIDC, cookie, WKWebView, or WebView2 row.

## Architecture boundary

| Surface | Production design | Status gate |
| --- | --- | --- |
| Web Workbench | Native Authentik public client | real two-client SSO suite |
| Web Business | Independent native client and session | real two-client SSO suite |
| macOS Workbench | System-browser OIDC code + PKCE | packaged Tauri acceptance |
| macOS Business | Gateway Embed → HttpOnly session | packaged WKWebView acceptance |
| Windows | Same protocol via WebView2 | NOT TESTED — RELEASE BLOCKED |

SQLite and the Node in-memory session map are POC-only. The Rust/PostgreSQL
Gateway is the V3.2 authority.

## Platform matrix

| Scenario | Web / Playwright Chromium | macOS 26.5.2 / Tauri WebKit | Windows / Tauri WebView2 |
| --- | --- | --- | --- |
| Workbench OIDC login | PASS | PASS | NOT TESTED |
| Authentik SSO established | PASS | PASS — system browser session survived app restart | NOT TESTED |
| Business native OIDC | PASS | N/A — not the selected Desktop transport | NOT TESTED |
| Device identity binding | N/A | PARTIAL — signed Gateway flow passes integration/unit tests; final packaged interaction pending | NOT TESTED |
| Embed Bootstrap issuance | PASS | PARTIAL — Gateway/Desktop policy passes; packaged interaction pending | NOT TESTED |
| Atomic redemption | PASS | PARTIAL — PostgreSQL and isolated-context tests pass; packaged WKWebView pending | NOT TESTED |
| No second password prompt | PASS | PASS — packaged macOS system-browser SSO reused | NOT TESTED |
| Business session cookie | PASS | PASS — packaged WKWebView accepted host-only Partitioned session | NOT TESTED |
| Business Dock authenticated | PASS | PASS — packaged Dock loaded dashboard and completed a CSRF write | NOT TESTED |
| Direct iframe OIDC redirect | FAIL — expected Authentik frame denial | N/A — prohibited | NOT TESTED |
| Native SSO as Business transport | PASS on Web | N/A — explicitly not selected | NOT TESTED |
| Replay / expiry / revocation rejection | PASS | PASS — shared PostgreSQL authority | NOT TESTED |
| Target / audience / deployment binding | PASS | PASS — shared PostgreSQL authority | NOT TESTED |
| Session expiry UI recovery | PARTIAL — static fixture | PASS — packaged 15s heartbeat detected natural expiry and completed one automatic Embed Session recovery without interaction or password | NOT TESTED |
| Business-only logout | PASS | PASS — packaged UI revoked only BusinessSession; Workbench remained active; SSO recovery required no password | NOT TESTED |
| Workbench-only logout | PASS — Gateway integration | PARTIAL — server cascade and toolbar unit path pass; packaged UI pending | NOT TESTED |
| Global logout | PASS — Gateway integration | PARTIAL — signed logout/cascade pass; packaged RP logout pending | NOT TESTED |
| Back-channel logout | PASS — signed sid and sub-only cases | PASS — shared Gateway authority | NOT TESTED |
| CSP / `frame-ancestors` | PASS | PARTIAL — exact configuration verified; native response pending | NOT TESTED |
| `SameSite=None; Secure; HttpOnly; Partitioned` | PASS | PASS — server and packaged WKWebView | NOT TESTED |
| App restart / cookie persistence | N/A | PARTIAL — Workbench SSO PASS; Business persistence pending | NOT TESTED |

## Automated and operational evidence

| Target | Result | Evidence |
| --- | --- | --- |
| Desktop full unit suite | PASS | 72 suites, 5152 tests |
| Static Business Dock fixture | PASS | 10/10 Playwright scenarios |
| V3.1 real two-client Web SSO | PASS | public Workbench and confidential Business clients; no second password prompt |
| V3.1 isolated one-time Embed Session | PASS | Authentik login, isolated redemption, HttpOnly cookie, target/replay/revocation checks |
| V3.2 Gateway unit tests | PASS | ticket entropy/hash/TTL and canonical binding proof |
| V3.2 JWT suite | PASS | valid token plus signature, issuer, audience/client, expiry, subject, sid and sub-only logout cases |
| V3.2 PostgreSQL security suite | PASS | binding, concurrency, replay, cascade, rate limit, logout, audit, cleanup, disabled user |
| Production Compose | PASS | clean image, migrations, readiness, explicit environment mapping, least-privilege audit grants |
| Clean macOS Relay bootstrap | PASS | isolated dependencies, bucket, migration, Relay readiness, packaged app launch |
| macOS packaged Business Dock | PASS | dashboard, CSRF write, Business-only logout/recovery and natural-expiry heartbeat recovery exercised in the rebuilt package |
| Windows packaged app | NOT TESTED — RELEASE BLOCKED | no Windows/WebView2 runner; follow `windows-auth-checklist.md` |

## Decision boundary

- Web: two independent native OIDC clients and application sessions.
- macOS: Workbench native SSO plus one-time Embed Session Bootstrap is the
  standard Business Dock path.
- Windows: `NOT TESTED — RELEASE BLOCKED`; do not claim production support
  before the exact packaged checklist passes.
