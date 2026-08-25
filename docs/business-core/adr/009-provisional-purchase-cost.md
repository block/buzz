# ADR 009: Provisional purchase cost

Status: accepted for B3.

Until supplier invoices and landed-cost policy exist, inventory uses the PO net
amount excluding tax as a provisional cost snapshot. B3 does not guess input-tax
deductibility. Later cost authorities adjust with explicit facts rather than
rewriting receipt or historical shipment costs.
