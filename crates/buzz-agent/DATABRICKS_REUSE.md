# Opt-in Databricks connection reuse

`databricks::DatabricksConnection` reuses the PKCE coordinator and v2 catalog
without running an agent, ACP, a CLI subprocess or any runtime configuration.

- Construct with an explicit HTTPS workspace origin, an absolute app-owned cache
  root and a `BrowserOpener`. No host default or environment credential lookup.
  Construction can read/create that cache, but never networks or opens a browser.
- `connect()` is user-initiated authentication only; it does not fetch models.
- `discover_models(filter)` is always headless, including one 401-driven refresh.
  It never selects a model. Existing filtering, partial-success and labelled
  authenticated-empty/no-filter fallback remain; a vector is not proof of a
  complete catalog.
- Strict discovery validates both OAuth endpoints from the very response used
  for refresh/code exchange. They must be same-origin HTTPS with no userinfo,
  query or fragment. Native discovery/token/catalog redirects are disabled.
  Browser SSO navigation is separate from the native client's redirect policy.
- The caller chooses and exclusively owns its cache root BEFORE construction.
  `databricks-strict` separates strict state from legacy `databricks` caches and
  single-flight coordination even if roots coincide. It is not credential
  migration. Existing host/client/scopes hash and Unix owner-only atomic token
  persistence remain. Non-Unix token persistence remains disabled.
- Keep secrets native. The supplied opener must not log authorization URLs or
  its failure details. Drop the operation future to cancel actual requests and
  the callback listener. Set an app-level overall deadline, generation-fence
  results, and never await under the agent-controller lock. A synchronous opener
  must return promptly; the library cannot cancel a blocking caller callback.
- A cache root is an application trust boundary, not protection against a
  malicious process with the same OS user or a caller lending out that root.

## Legacy compatibility and intentional shared changes

Existing public OAuth config/constructors, auth intents, cache defaults, static
bearer selection, CLI auth, desktop discovery and v1/v2 catalog APIs remain.
They DO NOT inherit strict same-origin or no-redirect policy.

Shared OAuth response handling has three intentional observable differences:

1. Discovery and successful token responses are stream-capped at **1 MiB**;
   OAuth error responses at **16 KiB**. Exact-boundary JSON succeeds; oversized,
   malformed or unreadable responses are infrastructure failures, never grant
   rejection. Only a client-error `error: invalid_grant` within the bound retains
   refresh/code rejection classification. Non-success discovery bodies are not
   consumed. Existing catalog limits/retries are unchanged.
2. OAuth logs no longer include raw response bodies, transport/decode details,
   callback error text or opener error text. Status/category logs remain. The
   legacy default opener still prints its manual sign-in URL deliberately; the
   strict API never selects that opener automatically. Strict catalog errors
   project fixed auth/infrastructure messages; legacy catalog diagnostics remain.
3. A token response whose `expires_in` overflows epoch arithmetic now fails as an
   infrastructure error instead of panicking/wrapping.

No old cache migration/deletion, credential rewrite, app launch, internal host,
internal release settings, or production dependency on a developer checkout is
part of this patch.

## Validation boundaries

Synthetic HTTPS servers use ephemeral test-only certificates; all cache contents,
refresh tokens and callback codes are fabricated. Tests cover new and legacy
paths, positive destination controls, refresh/code exchange, redirects, response
limits, classifications, captured diagnostics, cancellation/retry, cache roots
and namespace isolation. A real CLI child tests legacy auth aliases/cache reuse;
existing coordinator tests include real subprocess single-flight/locking.

Before integration, require independent compatibility review at an exact commit,
full `buzz-agent` package-suite evidence, affected desktop checks and repository
gates. Focused tests are not a substitute. App orchestration, internal build-input
wiring, credential-removal UX, native preview acceptance and real sign-in are
separate subsequent work, not claims made by this helper patch.
