# Managed-agent workflow delivery

A workflow `send_message` step has two separate outputs when it addresses a managed agent:

1. The relay signs and persists an ordinary kind `9` channel message. Its content is the rendered, human-visible step text. The event carries the channel, exact kind `30620` definition revision and step, workflow owner, semantic cause, and signed-template-derived `p` recipients. Private webhook fields and prior-step state are never copied into this event.
2. The relay creates a durable `workflow_agent_deliveries` row for each signed routing target, bound by foreign key to the exact partitioned kind `9` event. It then publishes an ephemeral kind `24620` wake hint containing only identifiers. Kind `24620` is `p`-gated; it is an accelerator rather than the source of truth.

The managed agent claims a delivery through the authenticated relay endpoint. Claiming is scoped to the host-derived community and authenticated agent pubkey, returns a fenced lease token, and includes the private trigger and execution snapshot. Before dispatch, ACP independently fetches and verifies the exact definition and visible message, immutable NIP-OA ownership, channel and workflow/run/step bindings, semantic cause, and locally rendered text. A stale or unverifiable delivery fails closed.

ACP also polls for pending deliveries, so an offline agent does not depend on receiving the ephemeral wake. Completion uses the lease token. Only an authenticated, token-fenced retryable finish returns a live claim to pending with bounded backoff. Once claimed, ACP is the sole retry authority: lease expiry is terminal and never redelivers, choosing at-most-once execution over potentially duplicating successful agent side effects after an uncertain finish or runtime exit. Pending rows that expire before claim also become terminal. The uniqueness key `(community_id, run_id, step_id, target_pubkey)` collapses a true replay while preserving distinct steps in the same run.
