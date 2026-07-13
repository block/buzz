-- Benchmark schema for harbor-buzz-orchestra runs.
--
-- Lives in the shared Postgres instance but is OWNED BY THE HARNESS, never by
-- Buzz migrations (canonical plan §two-domain rule). Idempotent: safe to apply
-- on every testbed bring-up.
--
--   docker exec -i <postgres> psql -U buzz -d buzz < sql/benchmark_schema.sql

CREATE SCHEMA IF NOT EXISTS benchmark;

-- One row per provisioned trial; written by BuzzTrialProvisioner.
CREATE TABLE IF NOT EXISTS benchmark.trial_manifest (
    run_id        text        NOT NULL,
    trial_id      uuid        NOT NULL,
    manifest_hash text        NOT NULL,
    channel_id    uuid        NOT NULL,
    created_at    timestamptz NOT NULL DEFAULT now(),
    handle        jsonb       NOT NULL DEFAULT '{}'::jsonb,
    archived_at   timestamptz,
    PRIMARY KEY (run_id, trial_id)
);

-- Immutable LLM receipts, ingested post-run from the accounting path
-- (Databricks AI Gateway inference tables first; LiteLLM shim fallback).
-- Authoritative for tokens/cost and orchestrator-vs-worker attribution;
-- Harbor AgentContext totals are a reconciliation checksum only.
CREATE TABLE IF NOT EXISTS benchmark.llm_receipts (
    receipt_id           bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    run_id               text        NOT NULL,
    trial_id             uuid        NOT NULL,
    agent_id             text        NOT NULL,
    role                 text        NOT NULL,
    condition            text        NOT NULL,
    endpoint             text        NOT NULL,
    model_revision       text        NOT NULL,
    request_id           text        NOT NULL,  -- gateway request identity
    requested_at         timestamptz NOT NULL,
    latency_ms           integer,
    input_tokens         bigint      NOT NULL DEFAULT 0,
    cached_input_tokens  bigint      NOT NULL DEFAULT 0,
    output_tokens        bigint      NOT NULL DEFAULT 0,
    cost_usd             numeric(12, 6),
    source               text        NOT NULL,  -- 'ai_gateway' | 'litellm' | 'client'
    raw                  jsonb       NOT NULL DEFAULT '{}'::jsonb,
    UNIQUE (source, request_id),
    FOREIGN KEY (run_id, trial_id)
        REFERENCES benchmark.trial_manifest (run_id, trial_id)
);

CREATE INDEX IF NOT EXISTS llm_receipts_trial_idx
    ON benchmark.llm_receipts (run_id, trial_id, agent_id);

-- Harness-recorded timing spans (monotonic clocks are authoritative for
-- latency). kind examples: 'trial', 'llm_call', 'terminal_exec',
-- 'terminal_queue_wait' — queue-wait is recorded separately from execution so
-- speed can be reported both as-run and queue-adjusted under the M1
-- serialized-broker policy.
CREATE TABLE IF NOT EXISTS benchmark.spans (
    span_id     bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    run_id      text        NOT NULL,
    trial_id    uuid        NOT NULL,
    agent_id    text,                            -- NULL for trial-level spans
    kind        text        NOT NULL,
    started_at  timestamptz NOT NULL,
    duration_ms bigint      NOT NULL CHECK (duration_ms >= 0),
    detail      jsonb       NOT NULL DEFAULT '{}'::jsonb,
    FOREIGN KEY (run_id, trial_id)
        REFERENCES benchmark.trial_manifest (run_id, trial_id)
);

CREATE INDEX IF NOT EXISTS spans_trial_kind_idx
    ON benchmark.spans (run_id, trial_id, kind);
