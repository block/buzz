# Business Read API

Production B2-B4/S1 reads for sales, purchasing, inventory, operational
receivables/payables, profit, management reports, the operating dashboard and
data quality are fetched from the
authoritative Business Core configured by `BUSINESS_CORE_BASE_URL` and
`BUSINESS_CORE_SERVICE_CREDENTIAL`. There is no runtime fallback to the bundled
acceptance dataset.

The bundled `desensitized-test-data` snapshot remains for the pre-existing V5
anomaly acceptance path and in-process tests; it is never a fallback for the
authoritative Business Core read tools.

Required runtime configuration is documented in
`docs/business-agent/real-business-api-integration.md` and `.env.example`.
`BUSINESS_AUTHORIZATION_ENABLED=true`, live Gateway delegation verification,
and Business Core's own actor permission/scope checks are mandatory; there is
no authorization or fixture bypass in a normal build.

The fixed route is `POST /v1/read/{tool}`. Fourteen authoritative read tools,
eight anomaly tools and six action-lifecycle reads form a fixed allowlist. The
process never exposes generic SQL, HTTP, arbitrary resource routes or writes.
