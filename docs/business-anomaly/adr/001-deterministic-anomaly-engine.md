# ADR 001: Deterministic anomaly engine

Status: accepted for V5 acceptance reference.

## Decision

Threshold comparison, money arithmetic, severity and cross-domain joins run in
versioned Rust rules. The LLM may explain a returned Finding and offer a review
suggestion, but cannot decide whether an anomaly exists.

## Rationale

Deterministic rules make boundary behavior, missing-data blocking, currency and
unit comparability, auditability and regression testing explicit. Stable ids
avoid false joins from names. Exact Evidence makes a conclusion reproducible.

## Consequences

Every rule needs configured thresholds and tests. Unsupported data produces
partial/low-confidence output rather than model completion. Customer-specific
policy changes require a new reviewed Rule Set Version.
