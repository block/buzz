# Buzz

Desktop chat shell with:

- Tauri + React + TypeScript + Vite
- Tailwind CSS
- shadcn/ui-ready shared components
- Biome (lint/format/check)
- Feature-driven frontend structure

## Scripts

- `pnpm dev` - run the web frontend
- `pnpm tauri dev` - run the desktop app
- `pnpm build` - typecheck and build frontend
- `pnpm typecheck` - TypeScript checks
- `pnpm lint` - Biome lint
- `pnpm format` - Biome format (write)
- `pnpm check` - Biome check

## Structure

- `src/shared` - reusable app-wide code (`ui`, `lib`, `styles`)
- `src/features` - feature modules (vertical slices)
- `src/app` - top-level app composition

## Square employee sign-in in staging

Square employee sign-in is disabled by default. A deployment can opt in for all
communities in the fixed staging namespace:

```sh
export BUZZ_EMPLOYEE_NATIVE_SIGN_IN=1
unset BUZZ_EMPLOYEE_NATIVE_RELAY_URL
export BUZZ_EMPLOYEE_NATIVE_RELAY_NAMESPACE=communities.square.builderlabstaging.xyz
```

Alternatively, retain the existing single-community configuration with
`BUZZ_EMPLOYEE_NATIVE_RELAY_URL=wss://<community-host>` and leave
`BUZZ_EMPLOYEE_NATIVE_RELAY_NAMESPACE` unset. Configuring both is an error. Existing
pilot launchers that export an exact relay remain in single-community mode until
the launcher configuration changes; installing a new binary does not change that
choice. These settings are read by the native process, not a browser flag.

Namespace mode accepts one canonical community label below that staging domain.
It rejects arbitrary or production namespaces, nested hosts, non-TLS URLs,
explicit ports and URL credentials. The Square browser and identity API origins
remain pinned to staging. Selecting a community does not authorize access: the
employee must complete their own Square sign-in, the server must validate their
current merchant/workspace access, and the existing signed own-key assertion must
succeed for that exact relay. Changing community or Buzz identity invalidates the
previous native binding; there is no owner-session fallback.

For a test, select the provisioned community through the normal desktop UI, then
choose **Sign in with Square**. Follow the generated browser URL unchanged to
preserve its fresh loopback callback and verifier-bound transaction. Sign in to
the intended Square merchant and continue back to Buzz. A static Dashboard URL
cannot complete the native transaction. Browser flags, approved service routes,
workspace grants and any explicit staging routing selector must already be
configured. Credentials and native callback codes must stay in the normal flow.
The namespace option does not renew or extend the short-lived Square session.

### Renewing employee access

**Renew Square session** opens the same Square browser handoff and obtains a new
short-lived server session. It is available in Square team member settings while
connected and on the recovery screen after expiry. Continue with the same Square
account and merchant. Buzz checks the prior subject, organization, workspace,
relay, and device key before replacing the active session. A failed or canceled
renewal leaves an existing session usable until its original expiry. To choose a
different account, explicitly sign out of Square first.

Expiry still discards the native credential, cancels protected relay access, and
hides protected content. Only the public continuity binding remains in memory;
it cannot authorize requests and is cleared by sign-out or a community/identity
change. After restarting the app, use normal Square sign-in. Renewal is
browser-assisted: it does not extend session lifetimes, reuse an expired
credential, or provide silent renewal after long idle periods. Silent renewal
requires a separately supported upstream authority contract.

To validate, renew while a session is active and verify that the same employee
can continue chatting. Cancel or reject a renewal and verify that the original
session still works until expiry. Then leave the app idle past expiry, return,
and confirm protected chat is hidden and renewal is available. Complete renewal
with the same account; the existing community, device key, and saved draft should
remain. A different account must be rejected without replacing the original
session. These checks require a human acceptance run before marking the PR ready.
