# Real Business API integration

## Current decision

Business Core is the authoritative production adapter for B2-B4 and S1 sales,
purchasing, inventory, operational receivables/payables, management profit,
operating dashboards and data quality. It is configured through
`BUSINESS_CORE_BASE_URL` and `BUSINESS_CORE_SERVICE_CREDENTIAL`; production has
no fallback to the acceptance dataset.

The bundled anomaly acceptance source is
`trade-acceptance-desensitized-v1`, classified
`desensitized-test-data`, with `dataAsOf` attached to every fact. It exercises
the same boundary a real adapter must use:

```text
MCP -> POST /v1/read/{fixed-tool} -> service authentication
-> verify active delegation/binding/user -> resolve authoritative scope
-> intersect requested scope -> read or deterministic analysis
-> verify again -> minimal result
```

It must never be presented as authoritative business data. Business Core reads
and acceptance anomaly fixtures remain explicitly separated.

## Formal permission dimensions

The reference authorization adapter resolves legal entity, warehouse,
customer, supplier, brand and business unit scopes. Requested values can only
narrow that set. Unknown users fail closed. Exact-id unauthorized and missing
objects both return 404.

Acceptance users are intentionally deterministic:

- finance: `aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaa1`, LE-A/LE-B and broad scopes;
- sales: `bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbb1`, LE-A/W01/C001-C002/S001/BR-A.

These UUIDs are test principals, not real people. Automated tests prove the two
users receive different scope hashes and results and that the sales user cannot
distinguish forbidden `SO-B-001` from an absent id.

## Fact and quality contract

Each fact carries source system, stable object id/version, update time and data
snapshot. Money and quantity values are decimal strings. Free-form source text
is not promoted into returned fact or Finding types. Missing profit components
produce `partial`, low confidence and `MISSING_COST_OR_EXPENSE`; stale snapshots
produce `STALE_SOURCE_DATA`.

The adversarial acceptance source contains instruction-like notes and a
`javascript:` link. A regression test proves none appear in the anomaly result.

## Production cutover gate

Do not mark READY until all of the following have runtime evidence:

1. named authoritative systems and endpoint versions;
2. non-shared production workload identity and secret rotation;
3. authoritative permission resolution for at least three dimensions;
4. two real test users with different scopes and an exact-id denial test;
5. actual snapshot freshness/SLA and source failure behavior;
6. Agent-to-Buzz response publication audited with the returned event id;
7. macOS and Windows Business Dock opening ResourceRefs against the same system.
