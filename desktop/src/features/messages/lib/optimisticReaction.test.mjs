import assert from "node:assert/strict";
import test from "node:test";

import { createOptimisticReaction } from "./optimisticReaction.ts";
import { formatTimelineMessages } from "./formatTimelineMessages.ts";
import { sortMessages } from "./messageQueryKeys.ts";
import { isDuplicateReactionError } from "@/features/pulse/lib/noteActions";
import { KIND_REACTION } from "@/shared/constants/kinds";

const CHANNEL_ID = "36411e44-0e2d-4cfe-bd6e-567eb169db9f";
const MESSAGE_ID =
  "08212d19af68b69e17ecfa4e2d4477a03f6aff70a3c3e2cf106b389a5a8f3515";
const ME = "1111111111111111111111111111111111111111111111111111111111111111";

function streamMessage() {
  return {
    id: MESSAGE_ID,
    pubkey: "2222222222222222222222222222222222222222222222222222222222222222",
    kind: 9,
    created_at: 1_700_000_000,
    content: "hello world",
    tags: [["h", CHANNEL_ID]],
    sig: "sig",
  };
}

function render(events) {
  return formatTimelineMessages(events, null, ME, null, {});
}

function reactionRow(rows) {
  const row = rows.find((r) => r.id === MESSAGE_ID) ?? rows[0];
  return row.reactions ?? [];
}

test("optimistic reaction renders the reactor's own reaction with no relay round-trip", () => {
  // Reactions carry only an `e` tag, so the #h-scoped live subscription never
  // delivers a freshly-clicked reaction back — the optimistic cache write is
  // the only thing that makes the reactor's own ✅ appear. Without the fix the
  // reaction is invisible and re-clicking hits the relay's duplicate guard.
  const optimistic = createOptimisticReaction(MESSAGE_ID, "✅", undefined, ME);

  assert.equal(optimistic.kind, KIND_REACTION);
  assert.deepEqual(
    optimistic.tags.find((t) => t[0] === "e"),
    ["e", MESSAGE_ID],
  );

  const rows = render(sortMessages([streamMessage(), optimistic]));
  const reactions = reactionRow(rows);

  assert.equal(reactions.length, 1, "one reaction should render");
  assert.equal(reactions[0].emoji, "✅");
  assert.equal(reactions[0].count, 1);
  assert.equal(
    reactions[0].reactedByCurrentUser,
    true,
    "the optimistic reaction must be attributed to the current user",
  );
});

test("the real backfilled reaction collapses onto the optimistic one — no double count", () => {
  // When the genuine kind:7 later loads via the #e aux backfill it has a
  // different event id, but render dedupes by target:actor:emoji, so the two
  // must not stack into a count of 2.
  const optimistic = createOptimisticReaction(MESSAGE_ID, "✅", undefined, ME);
  const realReaction = {
    id: "0f11e83b0f11e83b0f11e83b0f11e83b0f11e83b0f11e83b0f11e83b0f11e83b",
    pubkey: ME,
    kind: KIND_REACTION,
    created_at: 1_700_000_005,
    content: "✅",
    tags: [["e", MESSAGE_ID]],
    sig: "realsig",
  };

  const rows = render(
    sortMessages([streamMessage(), optimistic, realReaction]),
  );
  const reactions = reactionRow(rows);

  assert.equal(reactions.length, 1);
  assert.equal(
    reactions[0].count,
    1,
    "must not double-count optimistic + real",
  );
  assert.equal(reactions[0].reactedByCurrentUser, true);
});

test("custom-emoji optimistic reaction carries the NIP-30 emoji tag", () => {
  const optimistic = createOptimisticReaction(
    MESSAGE_ID,
    ":party:",
    "https://example.test/party.png",
    ME,
  );

  assert.deepEqual(
    optimistic.tags.find((t) => t[0] === "emoji"),
    ["emoji", "party", "https://example.test/party.png"],
  );
  assert.equal(optimistic.content, ":party:");
});

test("duplicate-on-add is recognized so the optimistic write is kept, not rolled back", () => {
  // Tyler's exact repro: the relay already holds the reactor's ✅ (it never
  // came back over the `#h` live sub, so the cache didn't render it). Clicking
  // again writes the optimistic reaction, then `addReaction` rejects with the
  // relay's duplicate message. useToggleReactionMutation swallows precisely
  // this error so onError never runs and the optimistic reaction survives —
  // turning "duplicate but nothing visible" into a rendered ✅. This pins the
  // relay error string that swallow depends on; if it drifts, the bug returns.
  const relayDuplicate = new Error(
    "relay rejected event: duplicate: reaction already exists",
  );
  assert.equal(isDuplicateReactionError(relayDuplicate), true);

  // Any other failure must still propagate (onError rolls back) — we only
  // treat duplicate-on-add as idempotent success.
  assert.equal(
    isDuplicateReactionError(new Error("relay rejected event: rate-limited")),
    false,
  );
});
