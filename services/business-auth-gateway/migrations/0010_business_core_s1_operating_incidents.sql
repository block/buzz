-- Business Core S1.4: operating-report incident lifecycle.
-- This is an operational control surface only. It does not create accounting,
-- journal, ledger, tax, banking, invoice, or statutory-reporting records.

CREATE TABLE operating_report_incidents (
    id UUID PRIMARY KEY,
    scope_hash TEXT NOT NULL CHECK (scope_hash ~ '^[0-9a-f]{64}$'),
    alert_code TEXT NOT NULL CHECK (alert_code ~ '^[A-Z][A-Z0-9_]{2,95}$'),
    severity TEXT NOT NULL CHECK (severity IN ('warning', 'critical')),
    message TEXT NOT NULL CHECK (char_length(message) BETWEEN 1 AND 500),
    evidence_path TEXT NOT NULL CHECK (evidence_path LIKE '/api/v1/%'),
    condition_status TEXT NOT NULL DEFAULT 'active'
        CHECK (condition_status IN ('active', 'cleared')),
    review_status TEXT NOT NULL DEFAULT 'open'
        CHECK (review_status IN ('open', 'acknowledged', 'in_progress', 'resolved')),
    assignee_user_id UUID REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    due_at TIMESTAMPTZ NOT NULL,
    occurrence_count BIGINT NOT NULL DEFAULT 1 CHECK (occurrence_count > 0),
    first_seen_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    cleared_at TIMESTAMPTZ,
    resolved_at TIMESTAMPTZ,
    created_by_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    last_trace_id UUID NOT NULL,
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (scope_hash, alert_code),
    CHECK (condition_status = 'cleared' OR review_status <> 'resolved'),
    CHECK (review_status <> 'resolved' OR resolved_at IS NOT NULL)
);

CREATE TABLE operating_report_incident_events (
    id UUID PRIMARY KEY,
    incident_id UUID NOT NULL REFERENCES operating_report_incidents(id) ON DELETE RESTRICT,
    event_type TEXT NOT NULL CHECK (event_type IN (
        'detected', 'condition_cleared', 'reopened', 'claimed',
        'acknowledged', 'started', 'due_changed', 'resolved'
    )),
    actor_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    trace_id UUID NOT NULL,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(payload) = 'object')
);

CREATE INDEX operating_report_incidents_scope_queue_idx
    ON operating_report_incidents(scope_hash, review_status, due_at);
CREATE INDEX operating_report_incidents_assignee_idx
    ON operating_report_incidents(assignee_user_id, review_status, due_at)
    WHERE review_status <> 'resolved';
CREATE INDEX operating_report_incident_events_timeline_idx
    ON operating_report_incident_events(incident_id, occurred_at DESC);

CREATE OR REPLACE FUNCTION operating_report_incident_event_append_only()
RETURNS TRIGGER AS $$
BEGIN
    RAISE EXCEPTION 'operating_report_incident_events is append-only';
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER operating_report_incident_events_no_update
    BEFORE UPDATE OR DELETE ON operating_report_incident_events
    FOR EACH ROW EXECUTE FUNCTION operating_report_incident_event_append_only();
