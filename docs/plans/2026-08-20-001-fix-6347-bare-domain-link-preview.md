---
title: "Fix #6347: Link Preview Rejects Bare-Domain URLs"
artifact_contract: ce-unified-plan/v1
artifact_readiness: implementation-ready
execution: code
product_contract_source: ce-plan-bootstrap
created: 2026-08-20
---

# Fix #6347: Link Preview Rejects Bare-Domain URLs Due to Trailing-Slash Mismatch

## Goal
Enable messages containing bare-domain HTTPS URLs (e.g. `https://api.airtable.com`) to send successfully when link previews are generated, without changing user-typed content.

## Context
- Issue: https://github.com/block/buzz/issues/6347
- Desktop `desktop/src/shared/lib/linkPreview.ts:272` uses `new URL(href).href` which adds `/` for bare domains. Relay `crates/buzz-relay/src/handlers/ingest.rs:259` requires `event.content.contains(canonical)` verbatim, so tag `https://api.airtable.com/` never matches body `https://api.airtable.com`.
- Suggested fixes: (1) desktop preserve original string, (2) relay tolerate slash variant. This plan does both for defense-in-depth.

## Decisions
- **KTD-1: Desktop normalizes bare domains to no-slash** — `createPreview()` returns `origin+search` when `pathname === "/"`. Rejected: preserve raw string with custom hash strip (more invasive, requires threading raw href through 5 call sites). Reason: one-line, localized, keeps existing URL parsing for other cases.
- **KTD-2: Relay tolerates both slash forms for root-path URLs** — new `content_contains_canonical()` checks alt variant (`/?` ↔ `?`, `/` ↔ ``). Rejected: only fix desktop. Reason: handles already-signed events and external producers.
- **KTD-3: No migration, no new kind, no config** — small bugfix, no schema change.

## Implementation Units

### U-1: Desktop bare-domain normalization
- Files: `desktop/src/shared/lib/linkPreview.ts`
- Change: In `createPreview()`, after `canonical.hash=""`, if `canonical.pathname === "/"` set `href = canonical.origin + canonical.search`.
- Tests: `desktop/src/shared/lib/linkPreview.test.mjs` (37 existing), plus manual probe for `https://api.airtable.com`, `https://api.airtable.com/?foo=1`, path and fragment cases.

### U-2: Relay tolerant containment check
- Files: `crates/buzz-relay/src/handlers/ingest.rs`
- Change: Add `content_contains_canonical(content, canonical_str, canonical)` and use it in `validate_link_preview_tags()`. Handles `/?` swap and trailing-slash toggle for `path === "/"`.
- Tests: Existing `link_preview_*` unit tests in `ingest.rs`, plus 8-case slash/query harness. No new migration.

## Dependencies & Sequencing
- U-1 and U-2 independent, can land together. No ordering constraint.

## Test Plan
- U-1: `pnpm exec tsx --test desktop/src/shared/lib/linkPreview.test.mjs` — must stay 37/37 green; manual: parse `https://api.airtable.com` → href without slash, `https://api.airtable.com/` → same, query and fragment preserved for non-bare paths.
- U-2: `cargo test -p buzz-relay --lib` link_preview tests (when network available) + offline Python harness covering both slash directions with/without query, non-root path negative case.
- Integration: Desktop e2e `messaging.spec.ts` link-preview preview not required for this fix, but manual compose `Request failed for https://api.airtable.com` → relay accepts (no 400).
- Formatting: `cargo fmt --all -- --check`, `biome check` for TS.

## Risks
- Low. Bare-domain only; path-bearing URLs untouched. Relay change is permissive (accepts superset), no tightening.

## Product Contract Preservation
- Product Contract unchanged — bugfix only.

## Confidence Check
- Plan is lightweight, 2 files, <40 lines. No external research needed.
