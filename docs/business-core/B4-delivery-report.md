# Business Core B4 Delivery Report

## 1. Executive Decision

`B4_CONDITIONALLY_READY`.

B4 code, migration, focused gate and disposable PostgreSQL workflow pass. The
decision cannot be raised to staging-ready while Gate 0 lacks real Authentik
browser-write, B2/B3 HTTP performance, and packaged macOS evidence.

## 2. Gate 0

- Browser BusinessSession + CSRF: middleware is reused by all B4 browser writes;
  a real Authentik browser session was not exercised in this run.
- B2/B3 HTTP write performance: not measured in this run.
- macOS packaged Business Dock: not run.
- Windows: no packaged acceptance run; no new platform-specific native code.

## 3. 利润事实

- Revenue: confirmed `shipment_lines.sales_amount`, already net of order-line discount.
- Product cost: frozen shipment `total_cost`/cost snapshot, never current average cost.
- Reversal: shipment reversal emits equal signed reversal facts.
- Projection: ordered outbox consumer with offset, retry rows and idempotent source key.
- Rebuild: resets the consumer offset and replays without deleting facts.
- Reconcile: compares signed facts with authoritative shipment revenue/cost per line.

## 4. 订单利润

Gross profit = net revenue − product cost. Contribution profit subtracts freight,
commission, platform fee, customer rebate and other direct cost, then adds supplier
rebate. Management operating profit subtracts allocated operating expense. Discount
is not subtracted again. Zero revenue returns null margins. Worker-disabled, stale or
failed projection data is Partial/Blocked rather than silently Complete.

## 5. 经营费用归集

Adjustment batches support draft/previewed/posted/reversed. Direct and rule-based
allocation share the same immutable preview. Post requires expected version, preview
id/hash and unchanged profit-fact watermark; otherwise `STALE_PREVIEW`. Reverse adds
reversal facts. These are management adjustments, not vouchers or journal entries.

## 6. 分配算法

Supported bases: direct, net revenue, product cost, shipped quantity and fixed
weight. Decimal weights are allocated at two-decimal currency precision. Largest
remainder resolves cents; sales-order UUID is the stable tie-break. Targets must
share legal entity/currency and pass current customer/brand/business-unit/warehouse
scope. Cross-entity and cross-currency allocation is rejected.

## 7. 盈利分析

Real fact aggregation supports customer, SKU, product category, brand, salesperson,
business unit, department, warehouse, sales order, legal entity and group, with an
optional distinct second dimension. One query uses one period and currency.

## 8. 管理报表

The current management report shows revenue, cost, gross/contribution/management
profit and explicitly unallocated draft/preview expense. Quality reflects worker,
staleness and unallocated expense. Currencies are never combined. Snapshots freeze
scope, rule, fact watermark, component totals and source hash; superseding creates a
new immutable snapshot. Every response carries the non-statutory warning.

## 9. Business Read API / MCP

Real B4 data now serves `query_order_profit`, `query_profitability_by_dimension`,
`get_management_profit_report`, `get_management_report_snapshot`,
`get_profit_evidence`, and `explain_profit_change`. All remain delegation-bound and
read-only; production has no B4 fixture fallback.

## 10. V5 异常分析

`PROFIT-LOSS-001`, `PROFIT-MARGIN-002`, `CROSS-LOSS-TERM-003` and the deterministic
profit bridge use Business Core B4 responses. Partial/blocked profit emits a data
quality finding instead of a definitive loss conclusion.

## 11. Business Web

Added order-profit list/detail, multidimensional profitability, operational
adjustment list/create/preview/reverse console, current management report and report
snapshot history/generation. Writes continue through BusinessSession/CSRF request
handling. Browser E2E against real auth remains Gate 0.

## 12. Business Dock

Allowlisted resources:

- `biz://order-profit/:sales_order_id`
- `biz://profitability/{customer|sku|brand|salesperson}/:id/:period`
- `biz://management-report/:snapshot_id`
- `biz://profit-adjustment/:batch_id`

All map to their `/embed` Business Web routes and reject query data/traversal.

## 13. 数据库

Migration `0008_business_core_b4_profitability_management_reporting.sql` adds
profit facts, projection offsets/failures, adjustment batches/lines/previews/
allocations/events, report snapshots/rows/evidence, lookup indexes, monetary/state
constraints and order-profit/reconciliation views. Facts, previews, allocations,
events and snapshot artifacts are append-only.

## 14. Audit 与 Outbox

Atomic audit/outbox events cover `PROFIT_FACT_PROJECTED`,
`PROFIT_FACT_REBUILD_COMPLETED`, `OPERATIONAL_ADJUSTMENT_CREATED`,
`OPERATIONAL_ADJUSTMENT_UPDATED`, `PROFIT_ALLOCATION_PREVIEWED`,
`OPERATIONAL_ADJUSTMENT_POSTED`, `OPERATIONAL_ADJUSTMENT_REVERSED`, and
`MANAGEMENT_REPORT_SNAPSHOT_GENERATED`.

## 15. 不变量与 Reconciliation

Verified: shipment revenue/cost project once; replay adds no duplicate facts;
normal+reversal signs reconcile; allocation sums exactly at cent precision; zero
weight fails closed; stale preview cannot post; concurrent post has one winner;
old snapshots retain their hash; fact/snapshot-row updates fail; reconciliation is
consistent after the full workflow.

## 16. 测试

- Unit/Rust/Clippy: PASS via `just business-b4-check`.
- PostgreSQL E2E: PASS on a fresh disposable PostgreSQL database; 0.72 s total.
- Projection/allocation/concurrency/idempotency/reversal/snapshot/reconciliation: PASS.
- Authorization: scoped service integration PASS; browser real-auth run not executed.
- CSRF: inherited middleware gate compiled/tested; real browser run not executed.
- Agent: MCP contract/retry/schema tests PASS; live delegation run not executed.
- Business Web TypeScript/build: PASS.
- Business Dock resolver: 16/16 PASS.
- Browser E2E, macOS package, Windows package: NOT RUN.

## 17. 性能

The disposable E2E dataset contains two shipped orders, four initial shipment facts
plus adjustment/reversal facts, and two snapshots. The complete DB workflow took
0.72 s with zero errors. Per-route HTTP P50/P95/max and production-scale results were
not measured; therefore no HTTP performance target is claimed and Gate 0 remains open.

## 18. 修改文件

- Runtime/schema: `.env.example`, `Justfile`, migration `0008`, Business Core B4
  module files, core router/config/main/common integration and PostgreSQL B4 test.
- Read/agent: `business-query-contracts`, `business-read-api`, `business-read-mcp`.
- UI/deep links: `apps/business-web` API/main/styles and Business Dock resolver/tests.
- Documentation: Business Core README, B2/B3 handoffs, B4 overview, eleven B4 topic/
  operations documents, ADR 012–016, and this report.

## 19. 上游 Buzz 影响

No Nostr kind, relay protocol, channel scope, thread counter or upstream Buzz HTTP
surface changed. The work remains in the existing Business Core/Read/MCP/Web/Dock
extension boundary. Desktop changes are resolver allowlist entries only.

## 20. 已知限制

Not implemented: electronic/supplier invoices, bank transactions, accounting/general
ledger, tax, statutory-profit reconciliation, cross-legal-entity group allocation,
FX conversion, formal rebate settlement, budgets and
forecasting. Report snapshot identity is fact-watermark based; draft unallocated
expense affects quality but does not create a new profit-fact watermark.

## 21. 后续方向（已被最新产品决策覆盖）

不启动 B5。下一阶段改为 **Business Core S1：核心稳固与经营驾驶舱**，集中关闭
B1–B4 的真实验收、对账、性能、可观测性和经营报表可用性；财务相关扩展暂停。
