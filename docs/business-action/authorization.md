# Authorization

Human reads and writes use an active BusinessSession. Writes additionally require an exact CSRF token, allowlisted Origin, capability permission, six-dimensional finding scope, current version, and rate limit. Unauthorized scoped entities return `not_found_or_forbidden` so existence is not disclosed.

Permissions are separate: finding read/acknowledge, proposal read, work-item create/update/assign/complete, and approval-draft create. Read access never implies create access. AgentReadDelegation adds only `business_action:read`; the MCP surface has six read tools and zero create/update/approval tools.

Acceptance mode recognizes only the explicitly seeded desensitized finance and sales identities. The sales identity lacks workflow write capabilities. No unknown identity is inferred from a Buzz name. Production has no fallback: missing formal authorization or directory resolver prevents service startup.
