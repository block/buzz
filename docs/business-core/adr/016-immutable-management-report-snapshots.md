# ADR 016: Immutable management-report snapshots

Accepted. Published management reports freeze scope, rule version, fact
watermark and source hash. Corrections create a new snapshot that may supersede
the old one; history is not overwritten. These snapshots preserve management
evidence but remain explicitly non-statutory because no general-ledger, invoice,
tax or legal-profit reconciliation exists.
