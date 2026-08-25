# Service Identity

The V5 shared-secret mode is not sufficient for a formal write path. The target
Staging design is short-lived workload identity or service JWT, protected with
mTLS where the platform supports it, with audience
`business-execution-preflight` for V6.5 and a distinct future execution
audience.

Required proof:

1. credential minted only to the named workload;
2. short lifetime and automated rotation;
3. immediate revocation test;
4. exact issuer/audience/service binding;
5. not forwarded to Agent, Desktop, Business Dock JavaScript or user session;
6. secret/header redaction in logs and audit.

No Staging workload identity or rotation/revocation evidence exists, so the
gate remains FAIL. Long-lived shared plaintext keys must not authorize V7.
