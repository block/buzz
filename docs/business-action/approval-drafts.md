# Approval drafts

An approval draft can be prepared only from an active work item whose catalog action declares a draft type. Preview and confirmation hashes bind it to the work-item/finding snapshot. States are only `draft`, `ready_for_review`, `withdrawn`, `expired`, and `superseded`; `draft_only` is always true.

There is no functional approve, reject, submit, execute, post, or ERP-sync operation. The acceptance page permanently states: “此内容仅为审批材料草稿，尚未提交正式审批，也不会触发任何业务操作。” It has no agree, reject, execute, or effective-state control.

Draft evidence stores IDs and snapshot hashes rather than credentials or executable payloads. It must not contain tokens, API URLs, database commands, payment/shipment instructions, journal entries, or a formal approval conclusion.
