# Business Approval Policy

`BusinessApprovalPolicy` is versioned and effective-dated. It records requester
and approver roles, count, self-approval, separation of duties, Step-up level,
expiry and a configuration hash. V6.5 parses policy only; it creates no Approval
Request or decision.

Policy invariants for future validation:

- medium risk defaults to no self-approval and separation of duties;
- high risk requires at least two approvers and is not eligible for the first pilot;
- Critical actions are excluded;
- candidate approvers come only from the authoritative directory and scope service;
- every future approval snapshot includes the exact policy version/hash.

No authoritative policy source or approver directory is configured, so this
condition is currently FAIL.
