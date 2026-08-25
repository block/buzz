# Business Core B1/B2/B3/B4/S1

Business Core is the authoritative Rust/Axum/SQLx modular monolith for one
customer group. B1 owns master data and authorization. B2 adds the real sales
order → reservation → shipment → inventory/cost → operational receivable →
  receipt/allocation loop. B3 adds purchase order → goods receipt → provisional
moving-average cost → operational payable → supplier payment/allocation. B4
adds shipment-based profit facts → deterministic operating-cost allocation →
multi-dimensional profitability → immutable management-report snapshots.
S1 adds scoped reconciliation health and a sales/purchasing/inventory/profit
operating dashboard without introducing a financial-accounting domain.
Production paths never read fixtures and do not add
tenant columns: one deployment/database belongs to one group.

## Ownership boundary

- `business-auth-gateway` remains authoritative for `EnterpriseUser`, Buzz
  identity bindings, embed sessions, and Business sessions.
- Business Core references those stable `enterprise_users.id` UUIDs for roles,
  scopes, salespeople, audit actors, assignees, and approvers. It never resolves
  a user from email or display name.
- The shared Business platform migration history lives in
  `services/business-auth-gateway/migrations`; migration `0005` adds B1.
- Business Core owns GroupProfile, LegalEntity, LedgerBook, all trade master
  data, roles, permissions, the six data-scope dimensions, candidate policies,
  its append-only audit log, and its outbox.
- Purchasing/payables are authoritative in B3. B4 is authoritative for the
  management-profit projection and operating adjustments; supplier invoices,
  accounting, tax and statutory profit remain outside Business Core.
- Inventory movements and allocation rows are append-only facts. Balances and
  open amounts are projections reconciled against those facts.

## Run

Run the Auth Gateway migration first or let Business Core run the same shared
migrator at startup. Both services must point at the same Business platform
database.

```bash
export BUSINESS_CORE_DATABASE_URL='postgres://...'
export BUSINESS_CORE_BIND_ADDR='127.0.0.1:3110'
export BUSINESS_CORE_SERVICE_AUTH_MODE='shared_secret'
export BUSINESS_CORE_SERVICE_AUDIENCE='business-core'
export BUSINESS_CORE_SERVICE_CREDENTIAL='<at least 32 bytes from a secret store>'
export BUSINESS_WEB_ORIGIN='https://business-staging.example.com'
export BUSINESS_WEB_EMBED_ORIGIN='https://business-staging.example.com'
cargo run -p business-core
```

Every route except `/health` requires these headers:

```text
x-business-service-credential: <service secret>
x-service-audience: business-core
x-enterprise-user-id: <active EnterpriseUser UUID>
x-trace-id: <UUID; optional but strongly recommended>
```

The caller is still checked against business roles and data scopes after
service authentication. Shared-secret auth is an internal deployment adapter,
not an end-user authentication mechanism. Place the service on a private
network and rotate the credential through the platform secret store.

## API

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/health` | Liveness only; no database readiness claim |
| `GET` | `/v1/group-profile` | Single authoritative group profile |
| `GET` | `/v1/master-data/{type}?limit=100` | Scope-filtered master-data directory |
| `GET` | `/v1/master-data/{type}/{id}` | Exact, scope-checked object lookup |
| `GET` | `/v1/authorization/users/{id}/roles` | Current roles and effective permissions |
| `GET` | `/v1/authorization/users/{id}/scopes` | Six current data-scope sets |
| `POST` | `/v1/authorization/access-check` | Permission plus exact-object scope decision |
| `POST` | `/v1/authorization/assignees/query` | Eligible responsible users from policy |
| `POST` | `/v1/authorization/approvers/query` | Eligible approvers, self-approval and step-up rules |
| `POST` | `/v1/admin/bootstrap` | One-time atomic B1 data load |
| `POST` | `/v1/admin/role-assignments` | Grant or revoke a role with expected revision |
| `POST` | `/v1/admin/scopes` | Grant or revoke one fixed scope dimension |
| `GET` | `/v1/sales-orders` | Scope-filtered authoritative orders for services |
| `GET` | `/api/v1/sales-orders/{id}/confirmation-preview` | Browser confirmation permission and live inventory readiness |
| `GET` | `/api/v1/shipments/draft-options` | Eligible reserved order lines for browser shipment entry |
| `GET` | `/api/v1/shipments/{id}/confirmation-preview` | Permission, inventory, moving-average cost and receivable release evidence |
| `GET` | `/api/v1/purchase-orders/entry-options` | Purchase-order create/update permission and optional scoped draft payload |
| `GET` | `/api/v1/purchase-orders/{id}/confirmation-preview` | Supplier, delivery, totals, line readiness and confirmation permission evidence |
| `GET` | `/api/v1/goods-receipts/draft-options` | Eligible purchase-order lines and remaining quantity for receipt entry |
| `GET` | `/api/v1/goods-receipts/{id}/confirmation-preview` | Provisional cost, moving-average, payable and permission evidence |
| `GET` | `/v1/inventory-balances` | Scope-filtered balance projections |
| `GET` | `/v1/trade-receivables` | Scope-filtered operational receivables |
| `GET` | `/v1/reconciliation/{inventory,receivables}` | Projection drift checks |
| `GET` | `/v1/purchase-orders` | Scope-filtered authoritative purchase orders |
| `GET` | `/v1/goods-receipts` | Goods receipts and provisional cost |
| `GET` | `/v1/trade-payables` | Operational payable projections |
| `GET` | `/v1/supplier-payments` | Supplier payment projections |
| `GET` | `/v1/reconciliation/payables` | Payable/payment drift checks |
| `GET` | `/v1/operations/incidents` | Scoped operating-report incident docket |
| `POST` | `/v1/operations/incidents/scan` | Detect, clear, or reopen current operating incidents |
| `POST` | `/v1/operations/incidents/{id}/commands` | Claim and transition an operating incident |
| `GET` | `/v1/order-profits` | Scoped real order-profit projection |
| `GET` | `/v1/profitability` | One- or two-dimensional profitability |
| `GET` | `/v1/management-profit-report` | Current non-statutory management report |
| `GET` | `/v1/management-report-snapshots[/{id}]` | Immutable report evidence |
| `GET` | `/v1/profit-evidence/{orderId}` | Source facts for an order |
| `GET` | `/v1/reconciliation/profit-facts` | Shipment/profit projection drift |
| `GET` | `/v1/operations/dashboard` | Scoped single-currency operating dashboard |
| `GET` | `/v1/operations/data-quality` | Cross-domain reconciliation and projection health |

Human B2/B3/B4 commands and S1 reads are under `/api/v1`. They require a valid
`__Host-bizfin_business` BusinessSession. Mutations additionally require an
exact configured Origin, CSRF header, `Idempotency-Key`, current expected
version, a fresh B1 authorization check and the command rate limit. Agent and
service credentials do not grant access to this browser write surface.

Supported `{type}` values are `legal_entity`, `ledger_book`, `business_unit`,
`department`, `unit_of_measure`, `product_category`, `brand`, `warehouse`,
`customer`, `supplier`, `product`, `sku`, and `salesperson`.

Authorization responses return `scopeVersion` and `effectiveScopeHash` so
downstream services can bind decisions and caches to an exact policy snapshot.
Unauthorized exact-object reads return `not_found_or_forbidden`; they do not
reveal whether the object exists.

## Controlled bootstrap

Bootstrap is disabled by default. To load a real Staging dataset, configure an
already active EnterpriseUser as the one bootstrap actor:

```bash
export BUSINESS_CORE_BOOTSTRAP_ENABLED=true
export BUSINESS_CORE_BOOTSTRAP_USER_ID='<active EnterpriseUser UUID>'
```

Submit one strict `BootstrapRequest` to `/v1/admin/bootstrap`, verify the
result, then immediately restore `BUSINESS_CORE_BOOTSTRAP_ENABLED=false` and
restart the service. The import is atomic and can succeed only while the group
profile is absent. It does not invent EnterpriseUsers: every referenced user
must already exist in Auth Gateway.

Normal role/scope mutations require `business_core:admin` and
`expectedAuthorizationRevision`. A stale revision returns HTTP 409. Each
successful mutation writes the append-only audit log and an outbox event in the
same database transaction.

## Verification

```bash
. ./bin/activate-hermit
just business-core-check
```

The PostgreSQL integration flows are opt-in and must use disposable databases:

```bash
BUSINESS_CORE_TEST_DATABASE_URL='postgres://.../business_core_test' \
  cargo test -p business-core --test postgres_b1 -- --nocapture

BUSINESS_CORE_B2_TEST_DATABASE_URL='postgres://.../business_core_b2_test' \
  cargo test -p business-core --test postgres_b2 -- --nocapture

BUSINESS_CORE_B3_TEST_DATABASE_URL='postgres://.../business_core_b3_test' \
  cargo test -p business-core --test postgres_b3 -- --nocapture

BUSINESS_CORE_B4_TEST_DATABASE_URL='postgres://.../business_core_b4_test' \
  cargo test -p business-core --test postgres_b4 -- --nocapture

just business-s1-check
```

It verifies the B1 staging shape (one group, two legal entities, two
warehouses, 20 SKUs, five customers, three suppliers, and three users), exact
scope allow/deny behavior, candidate resolution, optimistic authorization
revision checks, service authentication, append-only audit enforcement, and
the absence of `tenant_id`/`client_group_id`.

The B2 test exercises opening stock, idempotent posting, concurrent full
reservation, hold/release, partial shipment, moving-average cost snapshots,
receivable due dates, partial/full allocation, reversal ordering,
cancel-remaining, reconciliation, audit/outbox atomicity and append-only
enforcement. See [B2.md](../../docs/business-core/B2.md).

The B3 flow proves partial/final receipt allocation, moving-average quantity
and value, idempotency, concurrent over-receipt rejection, operational payable
recognition, supplier settlement/reversal, receipt reversal restrictions,
reconciliation, audit and outbox. See [B3.md](../../docs/business-core/B3.md).
