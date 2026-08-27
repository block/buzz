# Business Agent API

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

The fixed routes are `POST /v1/read/{tool}` and `POST /v1/write/{tool}`.
Fourteen authoritative reads, eight anomaly reads, six action-lifecycle reads
and six draft creates form the allowlist. Drafts pass through Business Core's
current-user permission and scope checks with a server-derived idempotency key.
The process never exposes generic SQL, HTTP, arbitrary resources, confirmation,
approval, reversal, posting or payment execution.

`BUSINESS_AGENT_DRAFT_WRITE_ENABLED` defaults to false. While false, the write
route returns not found without verifying or forwarding a delegation; reads
remain available.
