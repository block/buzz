# ADR 006: Operational receivable on shipment

Status: accepted for B2.

Shipment confirmation is completed fulfillment, so it creates the operational
receivable atomically with stock/cost movement. This aligns collections with
dispatch. The receivable is not a tax invoice, accounting subledger or general
ledger entry; later authorities must reference rather than redefine B2 facts.
