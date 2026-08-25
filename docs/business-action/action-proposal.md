# Action proposals

The service maps an active finding to compatible catalog entries and creates `suggested` proposals bound to finding ID/version/snapshot hash and rule-set version. A proposal may become `accepted`, `dismissed`, `expired`, or `superseded`; it never represents execution.

A material finding snapshot change supersedes old suggested proposals and generates new version-bound suggestions. Expired and dismissed proposals cannot create previews. Proposal content is assembled from catalog templates and bounded anomaly summaries, never from executable parameters, secrets, SQL, scripts, cookies, tokens, or raw free-text instructions.
