# Business analytics

Deterministic, read-only rule evaluation over normalized business facts.
`rules/trade-risk-v1.0.json` is the reviewed threshold source and
`fixtures/desensitized-v1.json` is explicitly desensitized acceptance data.

The crate has no network, database or model dependency. Joins use stable ids;
amounts use decimal strings. Incomplete data produces quality Findings and low
confidence rather than inferred values.
