import assert from "node:assert/strict";
import { test } from "node:test";

import { buildGroupedMembershipPayload } from "./membershipGroupPayload.ts";
import { buildTimelineItems } from "./timelineItems.ts";
import { KIND_SYSTEM_MESSAGE } from "../../../shared/constants/kinds.ts";

const DAY_START = Math.floor(Date.UTC(2026, 5, 14, 12, 0, 0) / 1000);

function systemEntry(id, createdAt, body) {
  return {
    message: {
      author: "System",
      body: JSON.stringify(body),
      createdAt,
      depth: 0,
      id,
      kind: KIND_SYSTEM_MESSAGE,
      pubkey: "aa".repeat(32),
      reactions: [],
      time: "12:00 PM",
    },
    summary: null,
  };
}

// One symbol per membership mechanism the relay can emit, so the matrix covers
// self-arrival, addition (same and different actor), and departure — for two
// distinct targets, which is what makes same-target vs cross-target grouping
// observable.
const SYMBOLS = {
  "self-join(a)": { type: "member_joined", actor: "aaa", target: "aaa" },
  "self-join(b)": { type: "member_joined", actor: "bbb", target: "bbb" },
  "add(a by x)": { type: "member_joined", actor: "xxx", target: "aaa" },
  "add(b by x)": { type: "member_joined", actor: "xxx", target: "bbb" },
  "add(b by y)": { type: "member_joined", actor: "yyy", target: "bbb" },
  "leave(a)": { type: "member_left", actor: "aaa" },
  "leave(b)": { type: "member_left", actor: "bbb" },
};

const SYMBOL_NAMES = Object.keys(SYMBOLS);

function sequencesOfLength(length) {
  if (length === 0) return [[]];
  return sequencesOfLength(length - 1).flatMap((prefix) =>
    SYMBOL_NAMES.map((name) => [...prefix, name]),
  );
}

function entriesFor(sequence) {
  return sequence.map((name, index) =>
    systemEntry(`e${index}`, DAY_START + index * 30, SYMBOLS[name]),
  );
}

/**
 * The seam this guards: `buildTimelineItems` owns *which* contiguous membership
 * events collapse into one `system-group`, and `buildGroupedMembershipPayload`
 * owns *how* that group is described. They are two statements of one rule.
 *
 * When the builder emits a group the payload builder returns null for,
 * `SystemMessageRow` falls back to the group's oldest message and every other
 * event in the group vanishes from the timeline — a member who left still
 * renders as present. That failure is silent, so it needs a standing guard
 * rather than one case-by-case regression per shape.
 */
test("every membership group buildTimelineItems emits is describable", () => {
  const undescribable = [];
  let groupsChecked = 0;

  for (const length of [2, 3, 4]) {
    for (const sequence of sequencesOfLength(length)) {
      const { items } = buildTimelineItems(entriesFor(sequence), null);
      for (const item of items) {
        if (item.kind !== "system-group") continue;
        groupsChecked += 1;
        const payload = buildGroupedMembershipPayload(
          item.entries.map((entry) => entry.message),
        );
        if (!payload) {
          undescribable.push(
            `${sequence.join(" → ")}  [group: ${item.entries
              .map((entry) => entry.message.id)
              .join(",")}]`,
          );
        }
      }
    }
  }

  assert.ok(groupsChecked > 0, "matrix produced no groups to check");
  assert.deepEqual(
    undescribable.slice(0, 10),
    [],
    `${undescribable.length} grouped shape(s) render as the oldest event only`,
  );
});

test("a lifecycle group needs every arrival to be a self-join by the departing member", () => {
  const elrond = "11".repeat(32);
  const viewer = "10".repeat(32);
  const message = (id, body) => ({
    author: "System",
    body: JSON.stringify(body),
    createdAt: 1,
    depth: 0,
    id,
    kind: KIND_SYSTEM_MESSAGE,
    reactions: [],
    time: "12:00 PM",
  });
  const selfJoin = (id) =>
    message(id, { type: "member_joined", actor: elrond, target: elrond });
  const leave = (id) => message(id, { type: "member_left", actor: elrond });

  // Accepted: one or more equivalent self-arrivals, then that member departing.
  for (const arrivals of [1, 2, 3]) {
    const messages = [
      ...Array.from({ length: arrivals }, (_, index) => selfJoin(`j${index}`)),
      leave("l"),
    ];
    assert.equal(
      buildGroupedMembershipPayload(messages)?.type,
      "member_joined_then_left",
      `${arrivals} self-arrival(s) + departure should be one lifecycle summary`,
    );
  }

  // Rejected: anything else. Each of these would misattribute a self-join the
  // member never made, or fold in an unrelated member's activity.
  const rejected = {
    "addition, not a self-join": [
      message("a", { type: "member_joined", actor: viewer, target: elrond }),
      leave("l"),
    ],
    "one arrival is an addition": [
      selfJoin("j0"),
      message("a", { type: "member_joined", actor: viewer, target: elrond }),
      leave("l"),
    ],
    "a different member departs": [
      selfJoin("j0"),
      message("l", { type: "member_left", actor: viewer }),
    ],
    "departure is not last": [leave("l"), selfJoin("j0")],
    "no departure at all": [selfJoin("j0"), selfJoin("j1")],
  };
  for (const [name, messages] of Object.entries(rejected)) {
    assert.notEqual(
      buildGroupedMembershipPayload(messages)?.type,
      "member_joined_then_left",
      `${name} must not render as a lifecycle summary`,
    );
  }
});
