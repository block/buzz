import assert from "node:assert/strict";
import test from "node:test";

import { groupBestieTodos } from "./bestieTodoGrouping.ts";
import {
  addBestieListItem,
  emptyBestieListState,
  reorderBestieTodos,
  toggleBestieListItemStarred,
} from "./bestieListStorage.ts";

const now = Math.floor(new Date(2026, 8, 27, 12, 0, 0).getTime() / 1000);

test("starred todos stay above day groups; Today first", () => {
  let state = emptyBestieListState();
  state = addBestieListItem(
    state,
    { dayKey: "2026-09-26", kind: "todo", text: "Yesterday item" },
    now,
  );
  state = addBestieListItem(
    state,
    { dayKey: "2026-09-27", kind: "todo", text: "Today item" },
    now + 1,
  );
  state = addBestieListItem(
    state,
    { dayKey: "2026-09-27", kind: "todo", text: "Star me" },
    now + 2,
  );
  const starId = state.items.find((i) => i.text === "Star me").id;
  state = toggleBestieListItemStarred(state, starId, now + 3);

  const grouped = groupBestieTodos(state.items, now);
  assert.equal(grouped.starred.length, 1);
  assert.equal(grouped.starred[0].text, "Star me");
  assert.equal(grouped.dayGroups[0].label, "Today");
  assert.ok(grouped.dayGroups[0].items.some((i) => i.text === "Today item"));
  assert.ok(
    !grouped.dayGroups.some((g) => g.items.some((i) => i.text === "Star me")),
  );
});

test("reorder across day groups updates dayKey and order", () => {
  let state = emptyBestieListState();
  state = addBestieListItem(
    state,
    { dayKey: "2026-09-26", kind: "todo", text: "Move me" },
    now,
  );
  const id = state.items[0].id;
  state = reorderBestieTodos(
    state,
    { dayKey: "2026-09-27", orderedIds: [id], starred: false },
    now + 1,
  );
  assert.equal(state.items[0].dayKey, "2026-09-27");
  assert.equal(state.items[0].starred, false);
  assert.equal(state.items[0].sortOrder, 0);
});
