import assert from "node:assert/strict";
import test from "node:test";

import {
  buildEditMentionState,
  replaceWithDraftMentionRefs,
  snapshotDraftMentionRefs,
} from "./draftMentionRefs.ts";
import { extractMentionPubkeys } from "./extractMentionPubkeys.ts";
import { formatTimelineMessages } from "./formatTimelineMessages.ts";
import { getSendToChannelSemantics } from "./sendToChannelSemantics.ts";
import { submitMessageEdit } from "../ui/submitMessageEdit.ts";

const A = "1".repeat(64);
const B = "2".repeat(64);
const C = "3".repeat(64);
const D = "4".repeat(64);
const E = "5".repeat(64);
const F = "6".repeat(64);
const AUTHOR = "a".repeat(64);
const OWNER = "b".repeat(64);
const OTHER = "f".repeat(64);
const EVENT = "e".repeat(64);
const original = (content, tags = []) => ({
  id: EVENT,
  pubkey: AUTHOR,
  kind: 9,
  created_at: 100,
  content,
  tags: [["h", "channel"], ...tags],
  sig: "fixture",
});
const edit = (content, tags = [], created_at = 101, pubkey = AUTHOR) => ({
  id: String(created_at).padStart(64, "0"),
  pubkey,
  kind: 40003,
  created_at,
  content,
  tags: [["h", "channel"], ["e", EVENT], ...tags],
  sig: "fixture",
});
const snapshot = (...keys) => [
  ["buzz:mention-snapshot"],
  ...keys.map((key) => ["mention", key]),
];
// These are accepted-event-shape semantic tests, not relay signature tests.
const render = (events, profiles) =>
  formatTimelineMessages(events, null, AUTHOR, null, profiles)[0];
const sorted = (keys) => [...keys].sort();

async function saveEdit({
  target,
  originalContent,
  content,
  members = [],
  selected,
  personas = [],
}) {
  const bindings = new Map();
  replaceWithDraftMentionRefs(target.mentionRefs, bindings, new Map());
  for (const [label, key] of selected ?? []) bindings.set(label, key);
  let saved;
  await submitMessageEdit({
    content,
    originalContent,
    editTarget: target,
    editTargetId: EVENT,
    ownerPubkey: AUTHOR,
    getMentionRefs: (text, fallback, competingDisplayNames) =>
      snapshotDraftMentionRefs(
        text,
        bindings,
        [],
        members,
        personas,
        fallback,
        competingDisplayNames,
      ),
    extractMentionPubkeys: (text, competingDisplayNames) =>
      extractMentionPubkeys({
        text,
        selectedMentions: bindings,
        selectedDisplayNames: personas,
        memberCandidates: members,
        competingDisplayNames,
      }),
    customEmoji: [],
    pendingImeta: [],
    queuedAttachments: [],
    spoileredAttachmentUrls: new Set(),
    clearComposer() {},
    restoreComposer() {},
    restoreMentionRefs() {},
    shouldRestoreComposer: () => true,
    setDeferredUploadPending() {},
    setUploadError(message) {
      throw Error(message);
    },
    revalidateMentionPubkeys: async (keys) => [...keys],
    save: async (body, refs, notifying) => {
      saved = { body, refs, notifying };
    },
  });
  assert.ok(saved, "edit must save");
  return saved;
}

for (const { name, labels, keys } of [
  {
    name: "two ambiguous aliases",
    labels: ["Scout", "Scout Jones"],
    keys: [
      [A, B],
      [C, D],
    ],
  },
  {
    name: "resolved short and ambiguous long",
    labels: ["Scout", "Scout Jones"],
    keys: [[A], [C, D]],
  },
  {
    name: "ambiguous short and resolved long",
    labels: ["Scout", "Scout Jones"],
    keys: [[A, B], [C]],
  },
  {
    name: "nested at-sign inside a historical label",
    labels: ["Scout", "Team @Scout"],
    keys: [
      [A, B],
      [C, D],
    ],
  },
  {
    name: "three overlapping ambiguous aliases",
    labels: ["Scout", "Scout Jones", "Scout Jones Senior"],
    keys: [
      [A, B],
      [C, D],
      [E, F],
    ],
  },
]) {
  const profiles = Object.fromEntries(
    labels.flatMap((displayName, i) =>
      keys[i].map((key) => [key, { displayName }]),
    ),
  );
  const originalContent = labels.map((label) => `@${label}`).join(" and ");
  for (const retained of [[labels.length - 1], [0], labels.map((_, i) => i)]) {
    test(`${name}: retain occurrences ${retained} with no current roster`, async () => {
      const content = `${retained.map((i) => `@${labels[i]}`).join(" and ")} edited`;
      const expected = sorted(retained.flatMap((i) => keys[i]));
      // Tag order must never select one of the tied keys as a literal binding.
      for (const delivered of [keys.flat(), keys.flat().reverse()]) {
        const source = original(
          originalContent,
          delivered.map((key) => ["p", key]),
        );
        let target = buildEditMentionState(
          source.content,
          source.tags,
          profiles,
          () => false,
        );
        for (const ref of target.mentionRefs) {
          assert.equal(keys[labels.indexOf(ref.displayName)].length, 1);
        }
        let message;
        // Save, real authorized overlay, reopen, then save/reopen again.
        for (let cycle = 0; cycle < 2; cycle++) {
          const saved = await saveEdit({
            target,
            originalContent: cycle ? message.body : originalContent,
            content: `${content}${cycle ? " again" : ""}`,
          });
          assert.deepEqual(sorted(saved.refs.map((tag) => tag[1])), expected);
          assert.ok(saved.notifying.every((key) => expected.includes(key)));
          const unresolvedExpected = sorted(
            retained.flatMap((i) => (keys[i].length > 1 ? keys[i] : [])),
          );
          assert.ok(
            saved.notifying.every((key) => !unresolvedExpected.includes(key)),
            "ambiguous historical refs never become edit notifications",
          );
          message = render(
            [
              source,
              edit(saved.body, [
                ...snapshot(),
                ...saved.refs,
                ...saved.notifying.map((key) => ["p", key]),
              ]),
              edit("unauthorized replacement", snapshot(OTHER), 103, OTHER),
            ],
            profiles,
          );
          assert.equal(message.body, saved.body);
          assert.equal(message.edited, true);
          target = buildEditMentionState(
            message.body,
            message.tags,
            profiles,
            () => false,
          );
          assert.deepEqual(
            sorted(target.unresolvedMentionPubkeys),
            unresolvedExpected,
          );
          assert.deepEqual(
            sorted(target.mentionRefs.map((ref) => ref.pubkey)),
            expected.filter((key) => !unresolvedExpected.includes(key)),
          );
          assert.deepEqual(
            sorted(getSendToChannelSemantics(message, profiles).mentionPubkeys),
            expected,
          );
        }
      }
    });
  }
}

test("latest authorized snapshot permits new recipients; later stranger edit is ignored", () => {
  const message = render([
    original("old", [["p", A]]),
    edit("first", snapshot(B)),
    edit("latest", snapshot(C), 102),
    edit("stranger", snapshot(D), 103, OTHER),
  ]);
  assert.equal(message.body, "latest");
  assert.deepEqual(getSendToChannelSemantics(message).mentionPubkeys, [C]);
});

test("profile-derived owner is accepted through the real fifth argument", () => {
  const message = render(
    [
      original("old", [["p", A]]),
      edit("owner edit", snapshot(C), 102, OWNER),
      edit("stranger", snapshot(D), 103, OTHER),
    ],
    { [AUTHOR]: { ownerPubkey: OWNER } },
  );
  assert.equal(message.body, "owner edit");
  assert.deepEqual(getSendToChannelSemantics(message).mentionPubkeys, [C]);
});

test("empty latest snapshot excludes old refs and text-only full keys", () => {
  const message = render([
    original("old", [["p", A]]),
    edit("first", snapshot(C)),
    edit(`@Mallory (${D})`, snapshot(), 102),
  ]);
  assert.deepEqual(getSendToChannelSemantics(message).mentionPubkeys, []);
  assert.deepEqual(
    buildEditMentionState(message.body, message.tags, undefined, () => false)
      .mentionRefs,
    [],
  );
});

test("legacy edit still uses occurrence-aware delivered-p intersection", () => {
  const profiles = {
    [C]: { displayName: "Alice" },
    [D]: { displayName: "Mallory" },
  };
  const message = render(
    [original("@Alice", [["p", C]]), edit("@Mallory", [["mention", D]])],
    profiles,
  );
  assert.deepEqual(
    getSendToChannelSemantics(message, profiles).mentionPubkeys,
    [],
  );
});

for (const mode of [
  "typed-same-label",
  "selected-same-label",
  "typed-longer",
  "persona-longer",
]) {
  test(`historical competitors preserve current ${mode} behavior`, async () => {
    const profiles = {
      [A]: { displayName: "Scout" },
      [B]: { displayName: "Scout" },
    };
    const source = original("@Scout history", [
      ["p", A],
      ["p", B],
    ]);
    const target = buildEditMentionState(
      source.content,
      source.tags,
      profiles,
      () => false,
    );
    const label = mode.endsWith("longer") ? "Scout Jones" : "Scout";
    const content = `@${label} replacement`;
    const saved = await saveEdit({
      target,
      originalContent: source.content,
      content,
      members: mode.startsWith("typed")
        ? [{ displayName: label, pubkey: C, isMember: true }]
        : [],
      selected: mode.startsWith("selected") ? [[label, C]] : [],
      personas: mode.startsWith("persona") ? [label] : [],
    });
    const expected = mode.startsWith("persona") ? [] : [C];
    // Typed same-label is a current addition, not an explicit replacement of
    // the historical ambiguous reference; current picker selection *is* one.
    const refs = mode === "typed-same-label" ? [A, B, C] : expected;
    assert.deepEqual(sorted((saved.refs ?? []).map((tag) => tag[1])), refs);
    assert.deepEqual(saved.notifying, expected);
    const message = render(
      [source, edit(content, [...snapshot(), ...(saved.refs ?? [])])],
      profiles,
    );
    assert.deepEqual(
      sorted(getSendToChannelSemantics(message).mentionPubkeys),
      refs,
    );
  });
}

test("authorized overlay keeps automatic address metadata delivery-gated", () => {
  const message = render([
    original("old", [
      ["p", A],
      ["mention", A, "agent-address"],
      ["mention", B, "agent-address"],
    ]),
    edit("new", snapshot(C)),
  ]);
  assert.deepEqual(sorted(getSendToChannelSemantics(message).mentionPubkeys), [
    A,
    C,
  ]);
});
