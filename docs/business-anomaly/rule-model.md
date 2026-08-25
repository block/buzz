# Rule model

`trade-risk-v1.0` is a validated JSON configuration loaded at service startup.
Each Finding has a deterministic UUID, Rule id/version, type, severity,
confidence, observed value, threshold, unit, optional monetary impact,
Evidence, related ResourceRefs, `dataAsOf`, status warnings and Trace id.

Rules use decimal strings and exact stable ids. Severity is assigned by code;
the model cannot alter it. Missing inputs block the affected conclusion and
lower confidence. Strictness (`<`, `>`, `>=`) is part of each rule contract and
has boundary regression coverage. A configured default version that differs
from the loaded rule file fails startup.

Adding a rule requires a new stable Rule id, explicit data requirements and
threshold semantics, positive/negative/boundary/missing-data tests, and an
update to [rule-reference.md](rule-reference.md).
