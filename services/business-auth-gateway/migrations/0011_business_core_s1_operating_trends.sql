-- Business Core S1.5: immutable operating snapshots and in-Dock schedules.
-- These records are management-operating evidence only; they are not ledgers,
-- journals, tax records, invoices, or statutory financial statements.

CREATE TABLE operating_report_snapshots (
    id UUID PRIMARY KEY,
    cadence TEXT NOT NULL CHECK (cadence IN ('daily', 'weekly')),
    period_start DATE NOT NULL,
    period_end DATE NOT NULL,
    currency CHAR(3) NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    scope_hash CHAR(64) NOT NULL CHECK (scope_hash ~ '^[a-f0-9]{64}$'),
    payload JSONB NOT NULL CHECK (jsonb_typeof(payload) = 'object'),
    data_quality_status TEXT NOT NULL CHECK (data_quality_status IN ('complete', 'partial', 'blocked')),
    source_hash CHAR(64) NOT NULL CHECK (source_hash ~ '^[a-f0-9]{64}$'),
    generated_by_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    generated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    trace_id UUID NOT NULL,
    CHECK (period_end > period_start),
    UNIQUE (cadence, period_start, currency, scope_hash)
);

CREATE INDEX operating_report_snapshots_trend_idx
    ON operating_report_snapshots(scope_hash, cadence, currency, period_start DESC);

CREATE TABLE operating_report_subscriptions (
    id UUID PRIMARY KEY,
    owner_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    cadence TEXT NOT NULL CHECK (cadence IN ('daily', 'weekly')),
    currency CHAR(3) NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    utc_offset_minutes SMALLINT NOT NULL CHECK (utc_offset_minutes BETWEEN -720 AND 840),
    delivery_hour SMALLINT NOT NULL CHECK (delivery_hour BETWEEN 0 AND 23),
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'paused')),
    next_run_at TIMESTAMPTZ NOT NULL,
    last_run_at TIMESTAMPTZ,
    last_snapshot_id UUID REFERENCES operating_report_snapshots(id) ON DELETE RESTRICT,
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (owner_user_id, cadence, currency)
);

CREATE INDEX operating_report_subscriptions_due_idx
    ON operating_report_subscriptions(next_run_at)
    WHERE status = 'active';

CREATE TABLE operating_report_subscription_events (
    id UUID PRIMARY KEY,
    subscription_id UUID NOT NULL REFERENCES operating_report_subscriptions(id) ON DELETE RESTRICT,
    event_type TEXT NOT NULL CHECK (event_type IN ('created', 'paused', 'resumed', 'generated', 'failed')),
    actor_user_id UUID NOT NULL REFERENCES enterprise_users(id) ON DELETE RESTRICT,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    trace_id UUID NOT NULL,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(payload) = 'object')
);

CREATE INDEX operating_report_subscription_events_timeline_idx
    ON operating_report_subscription_events(subscription_id, occurred_at DESC);

CREATE TRIGGER operating_report_snapshots_append_only
    BEFORE UPDATE OR DELETE ON operating_report_snapshots
    FOR EACH ROW EXECUTE FUNCTION business_core_audit_append_only();

CREATE TRIGGER operating_report_subscription_events_append_only
    BEFORE UPDATE OR DELETE ON operating_report_subscription_events
    FOR EACH ROW EXECUTE FUNCTION business_core_audit_append_only();

CREATE TRIGGER operating_report_subscriptions_touch
    BEFORE UPDATE ON operating_report_subscriptions
    FOR EACH ROW EXECUTE FUNCTION business_core_touch_updated_at();
