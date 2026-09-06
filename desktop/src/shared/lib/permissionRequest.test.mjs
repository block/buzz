/**
 * Named test matrix for the `permissionRequest` sentinel parser.
 *
 * The card is a two-action contract: the sentinel carries EXACTLY the ruled
 * `allow_once` and `reject_once` options and nothing else. The harness enforces
 * this on the producer side (`select_card_actions`); these tests pin the parser
 * side — exact cardinality, bounded ids/labels, unique ids, and
 * `chosenOptionId` membership.
 *
 * Wire contract: the harness signs BARE JSON as the kind:9 event content —
 * no fence wrapper. Tests feed raw JSON strings matching that shape exactly.
 */
import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const mod = await import("./permissionRequest.js").catch(
  () => import("./permissionRequest.ts"),
);
const { extractPermissionRequest, isPermissionRequestSentinel } = mod;

// ── Fixtures (bare JSON as the harness emits) ────────────────────────────────

const PENDING_NORMAL = {
  v: 1,
  state: "pending",
  requestNonce: "a9f3b2c1-d4e5-4f6a-b7c8-d9e0f1a2b3c4",
  sessionId: "sess-abc",
  turnId: "turn-xyz",
  expiresAt: 1786206732,
  optionIds: ["opt-allow", "opt-deny"],
  labels: { "opt-allow": "Allow once", "opt-deny": "Deny" },
};

const RESOLVED_APPLIED = {
  v: 1,
  state: "resolved",
  requestNonce: "a9f3b2c1-d4e5-4f6a-b7c8-d9e0f1a2b3c4",
  originalEventId:
    "deadbeef0001deadbeef0002deadbeef0003deadbeef0004deadbeef0005dead",
  sessionId: "sess-abc",
  turnId: "turn-xyz",
  expiresAt: 1786206732,
  optionIds: ["opt-allow", "opt-deny"],
  labels: { "opt-allow": "Allow once", "opt-deny": "Deny" },
  outcome: "applied",
  chosenOptionId: "opt-allow",
};

const RESOLVED_TIMED_OUT = {
  v: 1,
  state: "resolved",
  requestNonce: "a9f3b2c1-d4e5-4f6a-b7c8-d9e0f1a2b3c4",
  originalEventId:
    "deadbeef0001deadbeef0002deadbeef0003deadbeef0004deadbeef0005dead",
  sessionId: "sess-abc",
  turnId: "turn-xyz",
  expiresAt: 1786206732,
  optionIds: ["opt-allow", "opt-deny"],
  labels: { "opt-allow": "Allow once", "opt-deny": "Deny" },
  outcome: "timed_out",
  chosenOptionId: null,
};

const RESOLVED_CANCELLED = {
  v: 1,
  state: "resolved",
  requestNonce: "a9f3b2c1-d4e5-4f6a-b7c8-d9e0f1a2b3c4",
  originalEventId:
    "deadbeef0001deadbeef0002deadbeef0003deadbeef0004deadbeef0005dead",
  sessionId: "sess-abc",
  turnId: "turn-xyz",
  expiresAt: 1786206732,
  optionIds: ["opt-allow", "opt-deny"],
  labels: { "opt-allow": "Allow once", "opt-deny": "Deny" },
  outcome: "cancelled",
  chosenOptionId: null,
};

const RESOLVED_REJECTED = {
  v: 1,
  state: "resolved",
  requestNonce: "a9f3b2c1-d4e5-4f6a-b7c8-d9e0f1a2b3c4",
  originalEventId:
    "deadbeef0001deadbeef0002deadbeef0003deadbeef0004deadbeef0005dead",
  sessionId: "sess-abc",
  turnId: "turn-xyz",
  expiresAt: 1786206732,
  optionIds: ["opt-allow", "opt-deny"],
  labels: { "opt-allow": "Allow once", "opt-deny": "Deny" },
  outcome: "rejected",
  chosenOptionId: null,
};

// ── Helper: bare JSON string as the harness emits ─────────────────────────────
// No fence, no prose — this is the exact kind:9 event content string.
function raw(payload) {
  return JSON.stringify(payload);
}

// ── Parse: happy-path fixtures — raw JSON strings ─────────────────────────────

describe("extractPermissionRequest — pending fixtures", () => {
  it("test_pending_normal_parses_correctly", () => {
    const result = extractPermissionRequest(raw(PENDING_NORMAL));
    assert.ok(result !== null, "should parse");
    assert.equal(result.state, "pending");
    assert.equal(result.requestNonce, "a9f3b2c1-d4e5-4f6a-b7c8-d9e0f1a2b3c4");
    assert.equal(result.sessionId, "sess-abc");
    assert.equal(result.turnId, "turn-xyz");
    assert.equal(result.expiresAt, 1786206732);
    assert.deepEqual(result.optionIds, ["opt-allow", "opt-deny"]);
    assert.deepEqual(result.labels, {
      "opt-allow": "Allow once",
      "opt-deny": "Deny",
    });
    // pending has no originalEventId, outcome, chosenOptionId
    assert.ok(!("originalEventId" in result));
    assert.ok(!("outcome" in result));
    assert.ok(!("chosenOptionId" in result));
  });

  it("test_pending_with_leading_whitespace_parses_correctly", () => {
    // trim() before parse — consistent with how relay may deliver content
    const result = extractPermissionRequest(`  ${raw(PENDING_NORMAL)}\n`);
    assert.ok(result !== null, "should parse with surrounding whitespace");
    assert.equal(result.state, "pending");
  });
});

describe("extractPermissionRequest — resolved fixtures", () => {
  it("test_resolved_applied_parses_correctly", () => {
    const result = extractPermissionRequest(raw(RESOLVED_APPLIED));
    assert.ok(result !== null, "should parse");
    assert.equal(result.state, "resolved");
    assert.equal(result.outcome, "applied");
    assert.equal(result.chosenOptionId, "opt-allow");
    assert.equal(
      result.originalEventId,
      "deadbeef0001deadbeef0002deadbeef0003deadbeef0004deadbeef0005dead",
    );
  });

  it("test_resolved_timed_out_parses_correctly", () => {
    const result = extractPermissionRequest(raw(RESOLVED_TIMED_OUT));
    assert.ok(result !== null, "should parse");
    assert.equal(result.state, "resolved");
    assert.equal(result.outcome, "timed_out");
    assert.equal(result.chosenOptionId, null);
  });

  it("test_resolved_cancelled_parses_correctly", () => {
    const result = extractPermissionRequest(raw(RESOLVED_CANCELLED));
    assert.ok(result !== null, "should parse");
    assert.equal(result.state, "resolved");
    assert.equal(result.outcome, "cancelled");
    assert.equal(result.chosenOptionId, null);
  });

  it("test_resolved_rejected_parses_correctly", () => {
    const result = extractPermissionRequest(raw(RESOLVED_REJECTED));
    assert.ok(result !== null, "should parse");
    assert.equal(result.state, "resolved");
    assert.equal(result.outcome, "rejected");
    assert.equal(result.chosenOptionId, null);
  });
});

// ── Parse: rejection cases ────────────────────────────────────────────────────

describe("extractPermissionRequest — rejection cases", () => {
  it("test_prose_only_returns_null", () => {
    // Ordinary kind:9 message (no sentinel) — must not parse
    assert.equal(extractPermissionRequest("just prose, no JSON"), null);
  });

  it("test_wrong_version_returns_null", () => {
    const bad = { ...PENDING_NORMAL, v: 2 };
    assert.equal(extractPermissionRequest(raw(bad)), null);
  });

  it("test_unknown_state_returns_null", () => {
    const bad = { ...PENDING_NORMAL, state: "unknown" };
    assert.equal(extractPermissionRequest(raw(bad)), null);
  });

  it("test_empty_optionIds_returns_null", () => {
    const bad = { ...PENDING_NORMAL, optionIds: [], labels: {} };
    assert.equal(extractPermissionRequest(raw(bad)), null);
  });

  it("test_single_optionId_returns_null", () => {
    // Exactly two are required — one is not a valid two-action card.
    const bad = {
      ...PENDING_NORMAL,
      optionIds: ["opt-allow"],
      labels: { "opt-allow": "Allow once" },
    };
    assert.equal(extractPermissionRequest(raw(bad)), null);
  });

  it("test_three_optionIds_returns_null", () => {
    // A third option (e.g. allow_always) must never produce an actionable card.
    const bad = {
      ...PENDING_NORMAL,
      optionIds: ["opt-allow", "opt-deny", "opt-always"],
      labels: {
        "opt-allow": "Allow once",
        "opt-deny": "Deny",
        "opt-always": "Always allow",
      },
    };
    assert.equal(extractPermissionRequest(raw(bad)), null);
  });

  it("test_duplicate_optionIds_returns_null", () => {
    const bad = {
      ...PENDING_NORMAL,
      optionIds: ["opt-allow", "opt-allow"],
      labels: { "opt-allow": "Allow once" },
    };
    assert.equal(extractPermissionRequest(raw(bad)), null);
  });

  it("test_optionId_exceeding_200_chars_returns_null", () => {
    const big = "x".repeat(201);
    const bad = {
      ...PENDING_NORMAL,
      optionIds: [big, "opt-deny"],
      labels: { [big]: "Allow once", "opt-deny": "Deny" },
    };
    assert.equal(extractPermissionRequest(raw(bad)), null);
  });

  it("test_requestNonce_exceeding_200_chars_returns_null", () => {
    const bad = { ...PENDING_NORMAL, requestNonce: "n".repeat(201) };
    assert.equal(extractPermissionRequest(raw(bad)), null);
  });

  it("test_extra_label_key_returns_null", () => {
    // labels must have exactly one entry per optionId — no unbounded extra keys.
    const bad = {
      ...PENDING_NORMAL,
      labels: {
        "opt-allow": "Allow once",
        "opt-deny": "Deny",
        "opt-extra": "Extra",
      },
    };
    assert.equal(extractPermissionRequest(raw(bad)), null);
  });

  it("test_label_exceeding_200_chars_returns_null", () => {
    const bad = {
      ...PENDING_NORMAL,
      labels: { "opt-allow": "x".repeat(201), "opt-deny": "Deny" },
    };
    assert.equal(extractPermissionRequest(raw(bad)), null);
  });

  it("test_missing_requestNonce_returns_null", () => {
    const { requestNonce: _, ...bad } = PENDING_NORMAL;
    assert.equal(extractPermissionRequest(raw(bad)), null);
  });

  it("test_fractional_expiresAt_returns_null", () => {
    // Frozen schema requires integer seconds
    const bad = { ...PENDING_NORMAL, expiresAt: 1786206732.5 };
    assert.equal(extractPermissionRequest(raw(bad)), null);
  });

  it("test_negative_expiresAt_returns_null", () => {
    const bad = { ...PENDING_NORMAL, expiresAt: -1 };
    assert.equal(extractPermissionRequest(raw(bad)), null);
  });

  it("test_non_finite_expiresAt_returns_null", () => {
    // JSON.stringify converts Infinity to null, so this tests null expiresAt
    const bad = { ...PENDING_NORMAL, expiresAt: null };
    assert.equal(extractPermissionRequest(raw(bad)), null);
  });

  it("test_optionId_without_label_returns_null", () => {
    // Every advertised optionId must have a label entry
    const bad = {
      ...PENDING_NORMAL,
      optionIds: ["opt-allow", "opt-extra"],
      labels: { "opt-allow": "Allow once", "opt-deny": "Deny" },
      // "opt-extra" has no label; "opt-deny" is an extra key
    };
    assert.equal(extractPermissionRequest(raw(bad)), null);
  });

  it("test_resolved_missing_originalEventId_returns_null", () => {
    const { originalEventId: _, ...bad } = RESOLVED_APPLIED;
    assert.equal(extractPermissionRequest(raw(bad)), null);
  });

  it("test_resolved_originalEventId_wrong_length_returns_null", () => {
    const bad = { ...RESOLVED_APPLIED, originalEventId: "tooshort" };
    assert.equal(extractPermissionRequest(raw(bad)), null);
  });

  it("test_resolved_originalEventId_uppercase_returns_null", () => {
    // Must be lowercase hex per HEX64_RE
    const bad = {
      ...RESOLVED_APPLIED,
      originalEventId:
        "DEADBEEF0001DEADBEEF0002DEADBEEF0003DEADBEEF0004DEADBEEF0005DEAD",
    };
    assert.equal(extractPermissionRequest(raw(bad)), null);
  });

  it("test_resolved_unknown_outcome_returns_null", () => {
    const bad = { ...RESOLVED_TIMED_OUT, outcome: "expired" };
    assert.equal(extractPermissionRequest(raw(bad)), null);
  });

  it("test_resolved_applied_with_null_chosenOptionId_returns_null", () => {
    // outcome === "applied" requires a non-null chosenOptionId
    const bad = { ...RESOLVED_APPLIED, chosenOptionId: null };
    assert.equal(extractPermissionRequest(raw(bad)), null);
  });

  it("test_resolved_applied_chosenOptionId_not_in_optionIds_returns_null", () => {
    // chosenOptionId must be one of the advertised optionIds
    const bad = { ...RESOLVED_APPLIED, chosenOptionId: "opt-nonexistent" };
    assert.equal(extractPermissionRequest(raw(bad)), null);
  });

  it("test_resolved_timed_out_with_nonnull_chosenOptionId_returns_null", () => {
    // outcome !== "applied" requires null chosenOptionId
    const bad = { ...RESOLVED_TIMED_OUT, chosenOptionId: "opt-allow" };
    assert.equal(extractPermissionRequest(raw(bad)), null);
  });

  it("test_invalid_json_returns_null", () => {
    assert.equal(extractPermissionRequest("{not valid json}"), null);
  });

  it("test_empty_string_returns_null", () => {
    assert.equal(extractPermissionRequest(""), null);
  });

  it("test_json_array_returns_null", () => {
    // Arrays are not sentinel objects
    assert.equal(extractPermissionRequest("[1,2,3]"), null);
  });

  it("test_json_null_returns_null", () => {
    assert.equal(extractPermissionRequest("null"), null);
  });

  it("test_json_number_returns_null", () => {
    assert.equal(extractPermissionRequest("42"), null);
  });
});

// ── Byte-unit boundary (producer/parser agreement) ──────────────────────────
// These pin the shared unit: UTF-8 bytes, identical to the harness
// (`SENTINEL_STRING_MAX_BYTES` / `SENTINEL_CONTENT_MAX_BYTES`). A prior split
// (Rust char scalars vs JS UTF-16 code units) let a producer-valid multibyte
// label be rejected here, publishing a card the desktop renders as raw JSON.

describe("extractPermissionRequest — byte-unit boundaries", () => {
  // "😀" is 1 JS char pair (length 2 as UTF-16 units it counts as 2), 4 UTF-8 bytes.
  // 50 of them = 200 UTF-8 bytes: exactly at the limit, must be ACCEPTED.
  it("test_multibyte_label_at_200_bytes_parses", () => {
    const label = "😀".repeat(50); // 50 * 4 = 200 UTF-8 bytes
    assert.equal(new TextEncoder().encode(label).length, 200);
    const ok = {
      ...PENDING_NORMAL,
      labels: { "opt-allow": label, "opt-deny": "Deny" },
    };
    const result = extractPermissionRequest(raw(ok));
    assert.ok(result !== null, "a 200-UTF-8-byte label must be accepted");
    assert.equal(result.labels["opt-allow"], label);
  });

  it("test_multibyte_label_over_200_bytes_returns_null", () => {
    const label = "😀".repeat(51); // 204 UTF-8 bytes
    const bad = {
      ...PENDING_NORMAL,
      labels: { "opt-allow": label, "opt-deny": "Deny" },
    };
    assert.equal(extractPermissionRequest(raw(bad)), null);
  });

  it("test_sessionId_over_200_bytes_returns_null", () => {
    // The adapter-supplied sessionId is the load-bearing hole: an oversized one
    // must be rejected, not published as an unrenderable card.
    const bad = { ...PENDING_NORMAL, sessionId: "s".repeat(201) };
    assert.equal(extractPermissionRequest(raw(bad)), null);
  });

  it("test_turnId_over_200_bytes_returns_null", () => {
    const bad = { ...PENDING_NORMAL, turnId: "t".repeat(201) };
    assert.equal(extractPermissionRequest(raw(bad)), null);
  });

  it("test_total_content_over_max_bytes_returns_null", () => {
    // A structurally-valid sentinel padded past MAX_CONTENT_BYTES via an extra
    // (ignored) field must be rejected before parsing — the total-content gate.
    const bad = { ...PENDING_NORMAL, pad: "x".repeat(5000) };
    const serialized = raw(bad);
    assert.ok(
      new TextEncoder().encode(serialized).length > 4096,
      "fixture must exceed MAX_CONTENT_BYTES to exercise the gate",
    );
    assert.equal(extractPermissionRequest(serialized), null);
  });

  it("test_total_content_with_whitespace_padding_over_max_bytes_returns_null", () => {
    // The gate measures RAW content, not the trimmed value. A structurally-valid
    // sentinel prefixed with whitespace whose RAW size exceeds MAX_CONTENT_BYTES
    // must be rejected — otherwise whitespace padding smuggles signed content
    // past the frozen total boundary. Mutation proof: gating `content.trim()`
    // instead of `content` makes this test pass the oversized payload and parse.
    const body = raw(PENDING_NORMAL);
    const padded = " ".repeat(5000) + body;
    assert.ok(
      new TextEncoder().encode(body).length <= 4096,
      "the trimmed body alone must be within MAX_CONTENT_BYTES",
    );
    assert.ok(
      new TextEncoder().encode(padded).length > 4096,
      "raw padded content must exceed MAX_CONTENT_BYTES to exercise the gate",
    );
    assert.equal(extractPermissionRequest(padded), null);
  });

  it("test_normal_bounded_content_well_under_max_bytes_parses", () => {
    // A normal bounded sentinel — every leaf within MAX_STRING_BYTES and the
    // fixed field set — carries ample headroom below MAX_CONTENT_BYTES, so it
    // must parse. Assert the fixture's byte size to make that headroom explicit
    // and prove the total-content gate is not over-tight for typical content.
    // (Adversarial per-field-valid input can still reach the gate: JSON escaping
    // expands control characters, so distinct 200-byte option IDs built from
    // e.g. U+0000/U+0001, repeated as optionIds and label keys, can push the
    // serialized shape over 4096 and correctly fail closed at the producer.)
    const serialized = raw(PENDING_NORMAL);
    const size = new TextEncoder().encode(serialized).length;
    assert.ok(
      size < 4096,
      `a normal bounded sentinel must be under MAX_CONTENT_BYTES (got ${size})`,
    );
    assert.ok(
      extractPermissionRequest(serialized) !== null,
      "normal bounded content must parse",
    );
  });
});

// ── isPermissionRequestSentinel ───────────────────────────────────────────────

describe("isPermissionRequestSentinel", () => {
  it("test_sentinel_pending_returns_true", () => {
    assert.equal(isPermissionRequestSentinel(raw(PENDING_NORMAL)), true);
  });

  it("test_sentinel_resolved_returns_true", () => {
    assert.equal(isPermissionRequestSentinel(raw(RESOLVED_APPLIED)), true);
  });

  it("test_prose_message_returns_false", () => {
    assert.equal(isPermissionRequestSentinel("Hello world"), false);
  });

  it("test_invalid_json_returns_false", () => {
    assert.equal(isPermissionRequestSentinel("{bad json"), false);
  });

  it("test_json_without_v1_returns_false", () => {
    // A valid JSON object that is not a sentinel
    assert.equal(
      isPermissionRequestSentinel('{"type":"normal_message"}'),
      false,
    );
  });
});

// ── Harness integration fixture ───────────────────────────────────────────────
// The exact bytes below live in the shared, checked-in fixture
// `crates/buzz-acp/tests/fixtures/sentinel_pending.json`. The Rust test
// `kind9_content_fixture_structural_invariants` asserts `build_sentinel_pending_payload`
// output is byte-equal to that file; this test asserts the Desktop parser accepts
// the same file. Because both sides consume ONE file, a producer-side wire change
// breaks the Rust byte-equality assertion — the literal can no longer silently drift.
// (serde_json serializes object keys in sorted BTreeMap order.)
describe("harness integration fixture", () => {
  const HARNESS_KIND9_CONTENT = readFileSync(
    fileURLToPath(
      new URL(
        "../../../../crates/buzz-acp/tests/fixtures/sentinel_pending.json",
        import.meta.url,
      ),
    ),
    "utf8",
  );

  it("test_harness_kind9_content_parses_to_pending_payload", () => {
    const result = extractPermissionRequest(HARNESS_KIND9_CONTENT);
    assert.ok(
      result !== null,
      "harness fixture must parse to a non-null payload",
    );
    assert.equal(result.state, "pending");
    assert.equal(result.v, 1);
    assert.equal(result.requestNonce, "test-nonce-fixture-abc123");
    assert.equal(result.expiresAt, 1700000300);
    assert.deepEqual(result.optionIds, ["opt-allow", "opt-reject"]);
    assert.equal(result.sessionId, "sess-fixture-001");
    assert.equal(result.turnId, "turn-fixture-xyz");
    // F2: fixture now carries description from params.subject
    assert.equal(
      result.description,
      "read a file",
      "fixture description must round-trip from Rust producer",
    );
  });

  it("test_harness_kind9_content_identified_as_sentinel", () => {
    assert.equal(isPermissionRequestSentinel(HARNESS_KIND9_CONTENT), true);
  });
});

// ── F2: description field — parser validation ─────────────────────────────────

describe("extractPermissionRequest — description field", () => {
  it("test_pending_with_description_string_parses_correctly", () => {
    const payload = { ...PENDING_NORMAL, description: "read /etc/hosts" };
    const result = extractPermissionRequest(raw(payload));
    assert.ok(result !== null, "pending with description must parse");
    assert.equal(result.description, "read /etc/hosts");
  });

  it("test_pending_with_null_description_parses_correctly", () => {
    const payload = { ...PENDING_NORMAL, description: null };
    const result = extractPermissionRequest(raw(payload));
    assert.ok(result !== null, "pending with null description must parse");
    assert.equal(result.description, null);
  });

  it("test_pending_without_description_field_parses_correctly", () => {
    // No description key at all — field is optional
    const result = extractPermissionRequest(raw(PENDING_NORMAL));
    assert.ok(result !== null, "pending without description field must parse");
    assert.ok(
      result.description === undefined || result.description === null,
      "absent description must be undefined or null",
    );
  });

  it("test_pending_with_overlong_description_is_rejected", () => {
    // Parser-side: an over-limit description in the wire payload is rejected
    // (producer truncates producer-side; a non-compliant producer is untrusted).
    const overlong = "a".repeat(201); // 201 bytes > MAX_STRING_BYTES=200
    const payload = { ...PENDING_NORMAL, description: overlong };
    const result = extractPermissionRequest(raw(payload));
    assert.equal(
      result,
      null,
      "pending with over-limit description must be rejected by parser",
    );
  });

  it("test_pending_with_non_string_description_is_rejected", () => {
    // Parser-side: a non-null non-string description is invalid
    const payload = { ...PENDING_NORMAL, description: 42 };
    const result = extractPermissionRequest(raw(payload));
    assert.equal(
      result,
      null,
      "pending with numeric description must be rejected",
    );
  });

  it("test_pending_with_markup_description_parses_as_text", () => {
    // Hostile markup in description — parser accepts it as-is; React renders as text
    const hostile = "<script>alert(1)</script>";
    const payload = { ...PENDING_NORMAL, description: hostile };
    const result = extractPermissionRequest(raw(payload));
    assert.ok(result !== null, "hostile markup description must parse");
    // The string round-trips unchanged — sanitization is React's job at render time
    assert.equal(result.description, hostile);
  });

  it("test_resolved_with_description_parses_correctly", () => {
    const payload = {
      ...RESOLVED_APPLIED,
      description: "read /etc/hosts",
    };
    const result = extractPermissionRequest(raw(payload));
    assert.ok(result !== null, "resolved with description must parse");
    assert.equal(result.description, "read /etc/hosts");
  });

  it("test_resolved_with_overlong_description_is_rejected", () => {
    const overlong = "b".repeat(201);
    const payload = { ...RESOLVED_APPLIED, description: overlong };
    const result = extractPermissionRequest(raw(payload));
    assert.equal(
      result,
      null,
      "resolved with over-limit description must be rejected",
    );
  });
});
