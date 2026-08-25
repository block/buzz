# Rule reference

All rules below use version `trade-risk-v1.0`.

| Rule | Deterministic condition | Required data |
|---|---|---|
| `PROFIT-LOSS-001` | contribution profit `< 0` | revenue and required cost/expense components |
| `PROFIT-MARGIN-002` | contribution margin `< 3%` | complete profit fact |
| `AR-SHIP-003` | overdue `>=60d`, outstanding `> CNY 10,000`, active shipment/order | exact customer id + AR + sales |
| `AR-UNINVOICED-004` | shipped amount greater than invoiced after `>7d` | sales shipment/invoice amounts |
| `AR-UNPAID-005` | invoiced greater than received after `>30d` | invoice/receipt amounts |
| `PO-PRICE-006` | current comparable price increase `>=10%` | same SKU, supplier, currency, unit; 3+ samples |
| `PO-UNINVOICED-007` | received greater than invoiced after `>30d` | receipt/invoice amounts |
| `PO-PAYMENT-008` | payment rate minus receipt rate `>20%` | both progress rates |
| `INV-AGING-009` | age `>=180d` and zero 90-day sales | inventory, recent sales, in-transit quantity |
| `INV-STOCKOUT-010` | available below confirmed open-order demand | exact SKU+warehouse |
| `INV-NEGATIVE-011` | on-hand or available `<0` | inventory balance |
| `DATA-QUALITY-001` | required profit field missing | profit component presence |
| `CROSS-LOSS-TERM-003` | loss order linked to overdue AR | exact customer id |
| `PROFIT-BRIDGE-001` | 11 named bridge effects plus unexplained difference | comparable two-period snapshots |

These thresholds are acceptance defaults, not recommendations for every
customer. Production owners must approve a versioned customer rule set.
