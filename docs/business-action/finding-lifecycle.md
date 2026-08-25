# Finding lifecycle

`finding_key` is a deterministic hash input composed from rule ID, primary resource type/ID, legal entity, and relevant warehouse/customer/supplier/brand/business-unit scope. It is never model-generated.

`condition_status` is `active` or `cleared`. Only a completed run with the same scope hash and comparable rule-set version may clear a missing finding; partial and failed runs never clear. A repeated finding preserves `first_seen_at`, advances `last_seen_at` and `occurrence_count`, and refreshes evidence/hash when its snapshot changes.

`review_status` is `unreviewed`, `acknowledged`, `in_progress`, `resolved`, `dismissed`, or `reopened`. Resolve requires a code and note. Dismiss requires a code, comment, and bounded review date. A resolved finding that hits again reopens. An active dismissed finding whose review date expires reopens. A cleared finding only updates the linked work item's source condition; it never completes or cancels that item.
