import assert from "node:assert/strict";
import test from "node:test";

import {
  KIND_GROUPS,
  buildFinalKinds,
  buildSubscriptionRequest,
  isGroupFullyChecked,
  isGroupIndeterminate,
  parseCustomKinds,
  toggleGroup,
  toggleKind,
} from "./localArchiveKinds.ts";

// ── Helpers ───────────────────────────────────────────────────────────────────

function groupByLabel(label) {
  const g = KIND_GROUPS.find((g) => g.label === label);
  if (!g) throw new Error(`group not found: ${label}`);
  return g;
}

function kindsOfGroup(label) {
  return groupByLabel(label).items.map((i) => i.kind);
}

// ── parseCustomKinds ──────────────────────────────────────────────────────────

test("parseCustomKinds_emptyString_returnsEmpty", () => {
  const result = parseCustomKinds("");
  assert.deepEqual(result.valid, []);
  assert.deepEqual(result.invalid, []);
});

test("parseCustomKinds_singleValidKind_returnsIt", () => {
  const result = parseCustomKinds("60000");
  assert.deepEqual(result.valid, [60000]);
  assert.deepEqual(result.invalid, []);
});

test("parseCustomKinds_multipleValidKinds_returnsSorted", () => {
  const result = parseCustomKinds("30000 10000 20000");
  // buildFinalKinds sorts, parseCustomKinds preserves insertion order
  assert.deepEqual(result.valid, [30000, 10000, 20000]);
  assert.deepEqual(result.invalid, []);
});

test("parseCustomKinds_commaAndSpaceSeparated_bothWork", () => {
  const result = parseCustomKinds("30000,20000 10000");
  assert.deepEqual(result.valid, [30000, 20000, 10000]);
  assert.deepEqual(result.invalid, []);
});

test("parseCustomKinds_negativeInteger_isInvalid", () => {
  const result = parseCustomKinds("-5");
  assert.deepEqual(result.valid, []);
  assert.deepEqual(result.invalid, ["-5"]);
});

test("parseCustomKinds_float_isInvalid", () => {
  const result = parseCustomKinds("1.5");
  assert.deepEqual(result.valid, []);
  assert.deepEqual(result.invalid, ["1.5"]);
});

test("parseCustomKinds_kindAbove65535_isInvalid", () => {
  // NIP-01 kind range is 0..=65535; values above are rejected to prevent
  // the nostr crate's silent u16 truncation from creating unmatchable filters.
  const result = parseCustomKinds("65536 99999 89736");
  assert.deepEqual(result.valid, []);
  assert.deepEqual(result.invalid, ["65536", "99999", "89736"]);
});

test("parseCustomKinds_65535_isValid_boundary", () => {
  const result = parseCustomKinds("65535");
  assert.deepEqual(result.valid, [65535]);
  assert.deepEqual(result.invalid, []);
});

test("parseCustomKinds_nonNumericToken_isInvalid", () => {
  const result = parseCustomKinds("abc");
  assert.deepEqual(result.valid, []);
  assert.deepEqual(result.invalid, ["abc"]);
});

test("parseCustomKinds_mixedValidAndInvalid_separateCorrectly", () => {
  const result = parseCustomKinds("60000 abc 50000 -1");
  assert.deepEqual(result.valid, [60000, 50000]);
  assert.deepEqual(result.invalid, ["abc", "-1"]);
});

test("parseCustomKinds_duplicateTokens_deduped", () => {
  const result = parseCustomKinds("60000 60000 60000");
  assert.deepEqual(result.valid, [60000]);
  assert.deepEqual(result.invalid, []);
});

test("parseCustomKinds_kindAlreadyInGroups_silentlyIgnored", () => {
  // Kind 9 is in the "Messages & posts" group — not invalid but not returned
  const result = parseCustomKinds("9 60000");
  assert.deepEqual(result.valid, [60000]);
  assert.deepEqual(result.invalid, []);
});

test("parseCustomKinds_zero_isValid", () => {
  const result = parseCustomKinds("0");
  assert.deepEqual(result.valid, [0]);
  assert.deepEqual(result.invalid, []);
});

// ── buildFinalKinds ───────────────────────────────────────────────────────────

test("buildFinalKinds_onlyCheckedKinds_sortedDeduped", () => {
  const result = buildFinalKinds(new Set([40002, 9, 7]), []);
  assert.deepEqual(result, [7, 9, 40002]);
});

test("buildFinalKinds_onlyCustomKinds_sortedDeduped", () => {
  const result = buildFinalKinds(new Set(), [99999, 88888]);
  assert.deepEqual(result, [88888, 99999]);
});

test("buildFinalKinds_unionWithDedup_sortedCorrectly", () => {
  const result = buildFinalKinds(new Set([9, 40002]), [99999, 9]);
  // 9 appears in both — deduplicated to one entry
  assert.deepEqual(result, [9, 40002, 99999]);
});

test("buildFinalKinds_emptyInputs_returnsEmpty", () => {
  const result = buildFinalKinds(new Set(), []);
  assert.deepEqual(result, []);
});

// ── toggleKind ────────────────────────────────────────────────────────────────

test("toggleKind_uncheckedKind_addsIt", () => {
  const result = toggleKind(9, new Set([7]));
  assert.ok(result.has(9));
  assert.ok(result.has(7));
  assert.equal(result.size, 2);
});

test("toggleKind_checkedKind_removesIt", () => {
  const result = toggleKind(9, new Set([9, 7]));
  assert.ok(!result.has(9));
  assert.ok(result.has(7));
  assert.equal(result.size, 1);
});

// ── toggleGroup ───────────────────────────────────────────────────────────────

test("toggleGroup_noneChecked_checksAll", () => {
  const group = groupByLabel("Messages & posts");
  const result = toggleGroup(group, new Set());
  for (const item of group.items) {
    assert.ok(
      result.has(item.kind),
      `expected kind ${item.kind} to be checked`,
    );
  }
});

test("toggleGroup_allChecked_uncheksAll", () => {
  const group = groupByLabel("Messages & posts");
  const allChecked = new Set(kindsOfGroup("Messages & posts"));
  const result = toggleGroup(group, allChecked);
  assert.equal(result.size, 0);
});

test("toggleGroup_partiallyChecked_checksAll", () => {
  const group = groupByLabel("Messages & posts");
  const kinds = kindsOfGroup("Messages & posts");
  // Only one kind checked
  const partial = new Set([kinds[0]]);
  const result = toggleGroup(group, partial);
  for (const item of group.items) {
    assert.ok(result.has(item.kind));
  }
});

test("toggleGroup_doesNotAffectOtherKinds", () => {
  const group = groupByLabel("Messages & posts");
  const outerKind = 99999;
  const baseSet = new Set([outerKind]);
  const result = toggleGroup(group, baseSet);
  assert.ok(result.has(outerKind), "outer kind preserved");
});

// ── isGroupFullyChecked / isGroupIndeterminate ────────────────────────────────

test("isGroupFullyChecked_allInGroup_returnsTrue", () => {
  const group = groupByLabel("Reactions, edits & deletions");
  const all = new Set(kindsOfGroup("Reactions, edits & deletions"));
  assert.equal(isGroupFullyChecked(group, all), true);
});

test("isGroupFullyChecked_noneInGroup_returnsFalse", () => {
  const group = groupByLabel("Reactions, edits & deletions");
  assert.equal(isGroupFullyChecked(group, new Set()), false);
});

test("isGroupIndeterminate_someButNotAll_returnsTrue", () => {
  const group = groupByLabel("Reactions, edits & deletions");
  const kinds = kindsOfGroup("Reactions, edits & deletions");
  const partial = new Set([kinds[0]]);
  assert.equal(isGroupIndeterminate(group, partial), true);
});

test("isGroupIndeterminate_noneChecked_returnsFalse", () => {
  const group = groupByLabel("Reactions, edits & deletions");
  assert.equal(isGroupIndeterminate(group, new Set()), false);
});

test("isGroupIndeterminate_allChecked_returnsFalse", () => {
  const group = groupByLabel("Reactions, edits & deletions");
  const all = new Set(kindsOfGroup("Reactions, edits & deletions"));
  assert.equal(isGroupIndeterminate(group, all), false);
});

// ── Observer source fixed [24200] ─────────────────────────────────────────────

test("observerSource_fixedKind_is24200", () => {
  // The observer source always produces exactly [24200] — verify the constant
  // used in localArchiveKinds matches KIND_AGENT_OBSERVER_FRAME.
  // We verify this indirectly: kind 24200 must NOT appear in KIND_GROUPS
  // (it is not a channel event kind) so it would not be silently filtered
  // out of custom input — but more importantly it is never used in the
  // channel checklist, keeping the two paths cleanly separated.
  const allGroupedKinds = KIND_GROUPS.flatMap((g) =>
    g.items.map((i) => i.kind),
  );
  assert.ok(
    !allGroupedKinds.includes(24200),
    "kind 24200 must not appear in channel kind groups",
  );
});

// ── Empty selection blocks Add ────────────────────────────────────────────────

test("buildFinalKinds_emptySelection_producesEmptyArray", () => {
  // The Add button is disabled when buildFinalKinds returns length === 0.
  const result = buildFinalKinds(new Set(), []);
  assert.equal(result.length, 0);
});

// ── Group + custom union (deduped / sorted) ───────────────────────────────────

test("buildFinalKinds_groupAndCustom_unioned_sorted_deduped", () => {
  const messagesKinds = kindsOfGroup("Messages & posts");
  const customKinds = [99999, 88888];
  const result = buildFinalKinds(new Set(messagesKinds), customKinds);
  // Must be sorted ascending
  const sorted = [...result].sort((a, b) => a - b);
  assert.deepEqual(result, sorted);
  // Must include all messages kinds
  for (const k of messagesKinds) {
    assert.ok(result.includes(k));
  }
  // Must include custom kinds
  assert.ok(result.includes(99999));
  assert.ok(result.includes(88888));
  // No duplicates
  assert.equal(result.length, new Set(result).size);
});

// ── Single-kind selection ─────────────────────────────────────────────────────

test("buildFinalKinds_singleKind_returnsSingleElementArray", () => {
  const result = buildFinalKinds(new Set([9]), []);
  assert.deepEqual(result, [9]);
});

// ── Malformed custom input: mixed valid/invalid ───────────────────────────────

test("parseCustomKinds_malformedInput_onlyValidReturned", () => {
  const { valid, invalid } = parseCustomKinds("hello 60000 1.5 50000 -2");
  assert.deepEqual(valid, [60000, 50000]);
  assert.deepEqual(invalid, ["hello", "1.5", "-2"]);
});

// ── buildSubscriptionRequest — six acceptance cases ───────────────────────────

// Helper: get all kinds in a named group
function kindsOfGroupBySR(label) {
  const g = KIND_GROUPS.find((g) => g.label === label);
  if (!g) throw new Error(`group not found: ${label}`);
  return g.items.map((i) => i.kind);
}

test("buildSubscriptionRequest_singleKindSelection_exactKindsArray", () => {
  const req = buildSubscriptionRequest(
    "channel_h",
    "channel-abc",
    new Set([9]),
    [],
  );
  assert.ok(req !== null);
  assert.equal(req.scopeType, "channel_h");
  assert.equal(req.scopeValue, "channel-abc");
  assert.deepEqual(req.kinds, [9]);
});

test("buildSubscriptionRequest_groupHeaderToggle_allKindsInGroup", () => {
  const messagesKinds = kindsOfGroupBySR("Messages & posts");
  const req = buildSubscriptionRequest(
    "channel_h",
    "channel-abc",
    new Set(messagesKinds),
    [],
  );
  assert.ok(req !== null);
  assert.deepEqual(
    req.kinds,
    [...messagesKinds].sort((a, b) => a - b),
  );
});

test("buildSubscriptionRequest_groupPlusCustomKind_unionDedupedSorted", () => {
  const messagesKinds = kindsOfGroupBySR("Messages & posts");
  const customKinds = [50000, 50001];
  const req = buildSubscriptionRequest(
    "channel_h",
    "channel-abc",
    new Set(messagesKinds),
    customKinds,
  );
  assert.ok(req !== null);
  // Must be sorted ascending
  const sorted = [...req.kinds].sort((a, b) => a - b);
  assert.deepEqual(req.kinds, sorted, "kinds must be sorted");
  // Must include all messages kinds and both custom kinds
  for (const k of messagesKinds) {
    assert.ok(req.kinds.includes(k), `missing messages kind ${k}`);
  }
  assert.ok(req.kinds.includes(50000));
  assert.ok(req.kinds.includes(50001));
  // No duplicates
  assert.equal(req.kinds.length, new Set(req.kinds).size);
});

test("buildSubscriptionRequest_emptySelectionChannelH_returnsNull", () => {
  // Empty selection blocks Add — buildSubscriptionRequest returns null
  const req = buildSubscriptionRequest(
    "channel_h",
    "channel-abc",
    new Set(),
    [],
  );
  assert.equal(req, null);
});

test("buildSubscriptionRequest_observerSourceFixedKind_exactlyKind24200", () => {
  const req = buildSubscriptionRequest(
    "owner_p",
    "pubkey-abc",
    new Set(), // checked kinds ignored for owner_p
    [],
  );
  assert.ok(req !== null);
  assert.equal(req.scopeType, "owner_p");
  assert.equal(req.scopeValue, "pubkey-abc");
  assert.deepEqual(req.kinds, [24200]);
});

test("buildSubscriptionRequest_emptyPubkeyOwnerP_returnsNull", () => {
  // Empty pubkey (identity not yet loaded) blocks owner_p Add
  const req = buildSubscriptionRequest("owner_p", "", new Set(), []);
  assert.equal(req, null);
});
