-- Add the exact delegated-relationship selector introduced by the O4
-- trusted-evidence contract repair. Existing selector fingerprints and floors
-- remain byte-for-byte unchanged.

ALTER TABLE authorization_invalidation_floors
    DROP CONSTRAINT authorization_invalidation_floors_selector_kind_check;

ALTER TABLE authorization_invalidation_floors
    ADD CONSTRAINT authorization_invalidation_floors_selector_kind_check
    CHECK (selector_kind IN (
        'principal_fingerprint',
        'nostr_key',
        'binding',
        'session',
        'domain',
        'policy_version',
        'delegated_owner',
        'delegated_relationship'
    ));
