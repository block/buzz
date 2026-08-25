-- Append-only evidence for every number that reaches a committed business record.

ALTER TABLE business_numbering_sequence_pools
    ADD COLUMN baseline_value BIGINT NOT NULL DEFAULT 0
        CHECK (baseline_value >= 0 AND baseline_value <= current_value);

UPDATE business_numbering_sequence_pools
SET baseline_value = current_value;

CREATE TABLE business_numbering_issuances (
    id BIGSERIAL PRIMARY KEY,
    rule_id UUID NOT NULL REFERENCES business_numbering_rules(id) ON DELETE RESTRICT,
    record_type TEXT NOT NULL,
    aggregate_id UUID NOT NULL,
    rendered_number TEXT NOT NULL CHECK (char_length(rendered_number) BETWEEN 1 AND 64),
    source TEXT NOT NULL CHECK (source IN ('governed', 'fallback')),
    scope_key TEXT NOT NULL,
    scope_code TEXT,
    period_key TEXT NOT NULL,
    sequence_value BIGINT NOT NULL CHECK (sequence_value > 0),
    issued_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (record_type, aggregate_id),
    UNIQUE (record_type, rendered_number)
);

CREATE INDEX business_numbering_issuances_recent
    ON business_numbering_issuances(issued_at DESC, id DESC);

CREATE INDEX business_numbering_issuances_pool
    ON business_numbering_issuances(rule_id, scope_key, period_key, source, sequence_value);

CREATE OR REPLACE FUNCTION business_numbering_issuance_append_only() RETURNS TRIGGER AS $$
BEGIN
    RAISE EXCEPTION 'business_numbering_issuances is append-only';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER business_numbering_issuances_no_mutation
    BEFORE UPDATE OR DELETE ON business_numbering_issuances
    FOR EACH ROW EXECUTE FUNCTION business_numbering_issuance_append_only();
