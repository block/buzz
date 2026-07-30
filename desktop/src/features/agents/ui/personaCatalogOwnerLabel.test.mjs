import assert from "node:assert/strict";
import test from "node:test";

import { resolveCatalogOwnerLabel } from "./PersonaCatalogDialog.tsx";

// ── null / undefined summary ──────────────────────────────────────────────────

test("test_null_summary_returns_community_member", () => {
  assert.equal(resolveCatalogOwnerLabel(null), "Community member");
});

test("test_undefined_summary_returns_community_member", () => {
  assert.equal(resolveCatalogOwnerLabel(undefined), "Community member");
});

// ── populated displayName ─────────────────────────────────────────────────────

test("test_display_name_present_returns_display_name", () => {
  assert.equal(
    resolveCatalogOwnerLabel({ displayName: "Alice", name: "alice" }),
    "Alice",
  );
});

test("test_display_name_present_without_name_returns_display_name", () => {
  assert.equal(resolveCatalogOwnerLabel({ displayName: "Alice" }), "Alice");
});

// ── empty / whitespace displayName with valid name ────────────────────────────

test("test_empty_display_name_falls_through_to_name", () => {
  assert.equal(
    resolveCatalogOwnerLabel({ displayName: "", name: "alice" }),
    "alice",
  );
});

test("test_whitespace_only_display_name_falls_through_to_name", () => {
  assert.equal(
    resolveCatalogOwnerLabel({ displayName: "   ", name: "alice" }),
    "alice",
  );
});

// ── both candidates absent / empty ────────────────────────────────────────────

test("test_both_null_returns_community_member", () => {
  assert.equal(
    resolveCatalogOwnerLabel({ displayName: null, name: null }),
    "Community member",
  );
});

test("test_both_empty_returns_community_member", () => {
  assert.equal(
    resolveCatalogOwnerLabel({ displayName: "", name: "" }),
    "Community member",
  );
});

test("test_both_whitespace_returns_community_member", () => {
  assert.equal(
    resolveCatalogOwnerLabel({ displayName: "  ", name: "\t" }),
    "Community member",
  );
});

test("test_display_name_absent_name_present_returns_name", () => {
  assert.equal(resolveCatalogOwnerLabel({ name: "alice" }), "alice");
});

test("test_display_name_null_name_present_returns_name", () => {
  assert.equal(
    resolveCatalogOwnerLabel({ displayName: null, name: "alice" }),
    "alice",
  );
});
