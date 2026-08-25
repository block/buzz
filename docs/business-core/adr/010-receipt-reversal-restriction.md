# ADR 010: Receipt reversal restriction

Status: accepted for B3.

Direct correction is safe only when the payable has no active settlement and
the legal-entity/warehouse/SKU has no later inventory movement. Otherwise a
reversal would rewrite the economic meaning of later moving-average issues, so
B3 fails closed and defers to return/cost-adjustment workflows.
