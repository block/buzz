# Production HTTPS

Production Desktop and Web require a public CA or enterprise CA installed in OS
and WebView trust stores. TLS verification remains enabled. The V3.1 loopback
OIDC proxy and local Caddy certificate are Development/Test-only and must not be
present in a production build.

Expose Gateway at the Business origin for host-only cookies. Forward only a
validated client IP and `X-Trace-Id`; never log authorization headers. Redact
the full bootstrap query, disable analytics there, use HSTS, and keep Gateway on
a private network. Client/database secrets live only in a secret manager and
never in `VITE_*` variables.
