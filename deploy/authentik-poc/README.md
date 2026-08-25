# Authentik SSO POC

This environment pins the current official Authentik Compose topology (PostgreSQL, server, worker) and adds a deliberately small Business Auth Mock. The mock is a separate confidential OIDC client, exchanges its own authorization code server-side, stores only an opaque session identifier in an HttpOnly cookie, and implements the V3.1 one-time Desktop bootstrap.

The `docker-compose.poc-provision.yml` overlay and `business-dock-v3.1.yaml` blueprint are local POC helpers only. They create the two clients, POC users, groups and S256 policy from explicit `.env` values. Production does not include this overlay and there are no default passwords.

## 1. Local names and TLS

Add these development-only entries to `/etc/hosts`:

```text
127.0.0.1 auth.bizfin.test workbench.bizfin.test business.bizfin.test
```

Install `mkcert`, then create the files expected by the optional TLS overlay:

```bash
mkdir -p certs
mkcert -install
mkcert -cert-file certs/bizfin.test.pem -key-file certs/bizfin.test-key.pem auth.bizfin.test workbench.bizfin.test business.bizfin.test
cp "$(mkcert -CAROOT)/rootCA.pem" certs/rootCA.pem
```

The certificate and key are local artifacts and are ignored by Git. Production must use organization-managed DNS and certificates.

## 2. Secrets and startup

```bash
cp .env.example .env
openssl rand -base64 36  # PG_PASS
openssl rand -base64 60  # AUTHENTIK_SECRET_KEY
docker compose -f docker-compose.yml -f docker-compose.local-tls.yml pull
docker compose -f docker-compose.yml -f docker-compose.local-tls.yml up -d
```

Open `https://auth.bizfin.test/if/flow/initial-setup/`. Do not commit `.env`, `data/`, or `certs/`.

## 3. Authentik clients

Create two applications through **Applications → Applications → Create with provider → OAuth2/OIDC**.

| Setting | Workbench | Business |
| --- | --- | --- |
| Client type | Public | Confidential |
| Grant | Authorization code | Authorization code |
| PKCE | S256 required | S256 required |
| Redirect URI | `https://workbench.bizfin.test/auth/callback` | `https://business.bizfin.test/auth/callback` |
| Logout redirect | `https://workbench.bizfin.test/` | `https://business.bizfin.test/` |
| Scopes | `openid profile` | `openid profile` |

Use strict redirect matching. Do not enable implicit flow or wildcard redirects. Copy only the Business client secret into local `.env`; Workbench is a public client and must not have a shipped secret. Configure Authentik's allowed origins for the exact Workbench Origin if the token endpoint requires CORS.

## 4. Workbench

Create `desktop/.env.local` (also untracked):

```text
VITE_OIDC_ISSUER=https://auth.bizfin.test/application/o/workbench/
VITE_OIDC_CLIENT_ID=<workbench-client-id>
VITE_OIDC_REDIRECT_URI=https://workbench.bizfin.test/auth/callback
VITE_OIDC_POST_LOGOUT_REDIRECT_URI=https://workbench.bizfin.test/
VITE_BUSINESS_APP_ORIGIN=https://business.bizfin.test
VITE_BUSINESS_APP_URL=https://business.bizfin.test/
```

Run `pnpm dev --host 0.0.0.0` in `desktop/`. Production packages use the strict Authentik redirects `buzz://auth/callback` and `buzz://auth/logout-callback`. Development packages built with `src-tauri/tauri.dev.conf.json` must instead use `buzz-dev://auth/callback` and `buzz-dev://auth/logout-callback`; the distinct scheme prevents macOS from delivering a local acceptance callback to an installed production Buzz app. The app opens the system browser and consumes callbacks through Tauri's matching deep-link registration.

## 5. Test order

1. Sign in to Workbench in a top-level browser context.
2. Open Business Dock. Its first `CHECK_AUTH` must return `AUTH_REQUIRED`; no token should appear in `postMessage`.
3. Choose **Open sign-in**. Authentik should reuse its session without asking for credentials, then Business writes its own cookie.
4. Return to Workbench and choose **Check again**. The iframe should report only subject and display name.
5. Test Business-only logout, Workbench-only logout, Authentik session logout, session expiry, refresh, and app restart separately.

The Workbench public client requests `offline_access`; its provider permits the
refresh-token grant with a 90-day rotating lifetime. Desktop persists only the
OIDC user/refresh-token record in the OS keyring. A first sign-in is required
after upgrading from builds that kept the OIDC user only in memory.

The static `business-dock-test.html` remains an automation fixture. It is not evidence that real Authentik SSO or WebView cookie behavior works.

The mock's Business cookie is `HttpOnly; Secure; SameSite=None` because a Tauri WebView is cross-site relative to `business.bizfin.test`. Modern runtimes may partition or block it anyway; that outcome belongs in the platform matrix and is not a reason to weaken the cookie.

## 6. V3.1 automated POC provisioning

Fill every `REPLACE_WITH_...` value in `.env`, including unique POC passwords and the Business confidential-client secret. Start the exact validated topology:

```bash
docker compose \
  -f docker-compose.yml \
  -f docker-compose.local-tls.yml \
  -f docker-compose.poc-provision.yml \
  up -d --build
```

Authentik discovers the mounted blueprint. Verify PostgreSQL, server and worker health plus `/-/health/ready/` before testing. The POC provisions:

- `poc-user`: `bizfin-finance`, `bizfin-business`
- `poc-admin`: `bizfin-admin`, `bizfin-finance`, `bizfin-business`
- Workbench public client: strict Web and `buzz://` callbacks, code flow only
- Business confidential client: strict Business callbacks, code flow only

The real recorded run verified the expected `poc-user` groups in both clients. No RBAC behavior is implemented.

## 7. Desktop local names

The packaged macOS POC uses the `.localhost` hostnames already present in the Caddyfile; the reserved suffix resolves to loopback without editing `/etc/hosts`:

```text
https://auth.bizfin.localhost
https://business.bizfin.localhost
https://workbench.bizfin.localhost
```

Create a separate ignored `desktop/.env.production.local` with the Workbench issuer, public client ID, `buzz-dev://auth/callback`, `buzz-dev://auth/logout-callback`, Business origin and home URL when building with `tauri.dev.conf.json`. Use `buzz://` only for a production-identity package. `VITE_OIDC_DESKTOP_PROXY_ORIGIN=http://localhost` is permitted only for this local certificate POC. The native command is restricted to exact token, userinfo and Workbench JWKS paths. Production must omit the proxy and use organization-trusted HTTPS directly.

Use `just business-dock-macos-build` for the packaged acceptance app. It enables the `authentik-poc` feature together with the development application identity and keyring namespace. The ordinary release build does not compile or register the local OIDC proxy. Build validation fails closed if a production build sees `.test`/`.localhost` origins, `buzz-dev://` callbacks, or the loopback proxy.

## 8. One-time Embed Session

After Business authenticates the system-browser request, `/auth/embed-login` redirects to the POC-only `buzz-dev://auth/business-bootstrap` callback with a 256-bit single-use code. Business stores only a SHA-256 hash in the migrated SQLite table. The code is bound to the authenticated session, `business-dock` audience, an allowlisted same-origin target and a 30-second expiry. The local POC client never allowlists the production `buzz://` scheme.

`/embed/bootstrap` consumes the record atomically, writes a new Business HttpOnly session directly to the consuming context and redirects to the bound target. Replay, expiry, revocation, wrong audience and unsafe targets fail closed. Responses use `no-store` and `no-referrer`; sensitive values are not logged.

## 9. Recorded test commands

Run the policy and real Authentik suites without printing `.env`:

```bash
node --test business-auth-mock/embed-session-policy.test.mjs
cd ../../desktop
pnpm typecheck
pnpm exec playwright test --config playwright.authentik.config.ts web-sso.spec.ts
pnpm exec playwright test --config playwright.authentik.config.ts embed-session.spec.ts
```

The Web suite expects the `.test` Business environment (`SameSite=Lax`); the packaged desktop/Embed suite expects `.localhost` with `SameSite=None`. Restart only the Business mock when switching those local modes. Never commit either populated environment file.
