# Work items

A user first prepares a ten-minute preview. Confirmation requires `draft_id`, `preview_hash`, `expected_finding_version`, and `Idempotency-Key`. The service revalidates the active session, CSRF, origin, scope, permission, proposal, assignee, expiry, consumption state, finding version, and snapshot hash.

Allowed lifecycle: `open -> in_progress -> ready_for_review -> completed`; open/in-progress may block, blocked may return to in-progress, and open/in-progress/blocked may cancel. A completed item can reopen only after explicit user confirmation. Events are append-only and include created, assigned, started, blocked, unblocked, ready-for-review, completed, cancelled, reopened, and source-condition changes.

The current `AssigneeResolver` is current-user-only plus catalog role validation. It never maps Buzz display names. One active item per finding/action is enforced in memory and by a partial unique database index. Work items track internal follow-up; they do not pause shipment, cancel purchase, change credit, or write any authority record.
