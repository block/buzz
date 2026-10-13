-- Web Push (buzz-web) leases use semantic subscription classes (dm, mention,
-- thread_reply, needs_action). 0012 only allowed the APNs interruption levels,
-- so every buzz-web lease failed with "lease constraint violation" and no
-- wake was ever enqueued. Widen both checks; existing values stay valid.
ALTER TABLE push_leases DROP CONSTRAINT IF EXISTS push_leases_max_class_check;
ALTER TABLE push_leases ADD CONSTRAINT push_leases_max_class_check CHECK (
    max_class IS NULL OR max_class IN (
        'silent','default','time_sensitive','urgent',
        'dm','mention','thread_reply','needs_action'
    )
);
ALTER TABLE push_wake_outbox DROP CONSTRAINT IF EXISTS push_wake_outbox_class_check;
ALTER TABLE push_wake_outbox ADD CONSTRAINT push_wake_outbox_class_check CHECK (
    class IN (
        'silent','default','time_sensitive','urgent',
        'dm','mention','thread_reply','needs_action'
    )
);
