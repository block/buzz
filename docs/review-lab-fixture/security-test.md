# Security review lab fixture — DO NOT MERGE

This disposable draft tests security lane routing and reviewer behavior. It changes
only documentation. The fenced pseudocode contains deliberate defects and must
never be copied into production. No real credentials or user data are included.

## Review areas

- [Authorization: account-scoped document access](security/document-access.md)
- [Sensitive data: request logging](security/request-logging.md)

## Routing

Configure a Security lane with either **Label is `security`** or **File path matches
`docs/review-lab-fixture/security/**`**, then assign Security Reviewer and save.
Submit this draft manually in the isolated lab. Reviews belong only in the lab's
Buzz threads; do not post automated reviews to GitHub or merge this draft.
