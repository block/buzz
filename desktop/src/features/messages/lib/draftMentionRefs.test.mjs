import assert from "node:assert/strict";
import test from "node:test";

import {
  buildEditMentionState,
  buildMessageComposerEditTarget,
  resolveEditMentionRefs,
} from "./draftMentionRefs.ts";

const ALICE = "a".repeat(64);
const BOB = "b".repeat(64);
const message = (body, tags) => ({
  author: "Alice",
  body,
  id: "message-id",
  tags,
});

const profiles = {
  [ALICE]: { displayName: "Alice" },
  [BOB]: { displayName: "Bob" },
};

test("edit mention refs resolve from visible text and loaded profiles", () => {
  assert.deepEqual(
    resolveEditMentionRefs(
      "Please review this, @Alice.",
      [["p", ALICE]],
      profiles,
      () => false,
    ),
    [{ displayName: "Alice", isAgent: false, pubkey: ALICE }],
  );

  const target = buildMessageComposerEditTarget(
    message("Please review this, @Alice.", [["p", ALICE]]),
    profiles,
    () => false,
  );
  assert.deepEqual(target.unresolvedMentionPubkeys, []);
});

test("shared edit mention state preserves tagged identities while profiles are unavailable", () => {
  assert.deepEqual(
    buildEditMentionState(
      "Please review this, @Alice and @Bob.",
      [
        ["p", ALICE],
        ["mention", BOB],
      ],
      undefined,
      () => false,
    ),
    { mentionRefs: [], unresolvedMentionPubkeys: [ALICE, BOB] },
  );
});

test("edit target preserves tagged identities while profiles are unavailable", () => {
  const target = buildMessageComposerEditTarget(
    message("Please review this, @Alice and @Bob.", [
      ["p", ALICE],
      ["mention", BOB],
    ]),
    undefined,
    () => false,
  );

  assert.deepEqual(target.mentionRefs, []);
  assert.deepEqual(target.unresolvedMentionPubkeys, [ALICE, BOB]);
});

test("edit target records semantic thread ownership", () => {
  const root = buildMessageComposerEditTarget(
    message("Root", [["h", "channel-id"]]),
    undefined,
    () => false,
  );
  const reply = buildMessageComposerEditTarget(
    message("Reply", [
      ["h", "channel-id"],
      ["e", "root-id", "", "root"],
      ["e", "root-id", "", "reply"],
    ]),
    undefined,
    () => false,
  );

  const broadcastReply = buildMessageComposerEditTarget(
    message("Broadcast reply", [
      ["h", "channel-id"],
      ["e", "root-id", "", "reply"],
      ["broadcast", "1"],
    ]),
    undefined,
    () => false,
  );

  assert.equal(root.isThreadReply, false);
  assert.equal(reply.isThreadReply, true);
  assert.equal(broadcastReply.isThreadReply, false);
});

test("edit target separates resolved refs from identities missing profiles", () => {
  const target = buildMessageComposerEditTarget(
    message("Please review this, @Alice and @Bob.", [
      ["p", ALICE],
      ["mention", BOB],
    ]),
    { [ALICE]: profiles[ALICE] },
    () => false,
  );

  assert.deepEqual(target.mentionRefs, [
    { displayName: "Alice", isAgent: false, pubkey: ALICE },
  ]);
  assert.deepEqual(target.unresolvedMentionPubkeys, [BOB]);
});

test("draft snapshots and explicit extraction agree on longest selected or typed occurrences", async () => {
  const { snapshotDraftMentionRefs } = await import("./draftMentionRefs.ts");
  const { extractMentionPubkeys } = await import("./extractMentionPubkeys.ts");
  for (const longer of ["Scout Jones", `Scout (${BOB})`, `Scout (${BOB}) 12`]) {
    const mentions = new Map([
      ["Scout", ALICE],
      [longer, BOB],
    ]);
    for (const text of [
      `@${longer}!`,
      `(@${longer})`,
      `**@${longer}**`,
      `@${longer}, hello`,
    ]) {
      assert.deepEqual(
        snapshotDraftMentionRefs(text, mentions, ["Scout"]).map(
          (ref) => ref.pubkey,
        ),
        [BOB],
      );
      assert.deepEqual(
        extractMentionPubkeys({
          text,
          selectedMentions: mentions,
          memberCandidates: [],
        }),
        [BOB],
      );
    }
  }
  const members = [{ displayName: "Scout Jones", pubkey: BOB, isMember: true }];
  assert.deepEqual(
    snapshotDraftMentionRefs(
      "@Scout Jones",
      new Map([["Scout", ALICE]]),
      ["Scout"],
      members,
    ),
    [],
  );
  assert.deepEqual(
    snapshotDraftMentionRefs("`@Scout`", new Map([["Scout", ALICE]]), []),
    [],
  );
});

test("historical ambiguous aliases fail closed and untagged qualified keys do not hydrate", () => {
  const same = {
    [ALICE]: { displayName: "Scout" },
    [BOB]: { displayName: "Scout" },
  };
  const ambiguous = buildEditMentionState(
    "@Scout hello",
    [
      ["p", ALICE],
      ["p", BOB],
    ],
    same,
    () => false,
  );
  assert.deepEqual(ambiguous.mentionRefs, []);
  assert.deepEqual(ambiguous.unresolvedMentionPubkeys, [ALICE, BOB]);
  const untrusted = buildEditMentionState(
    `@Other (${BOB})`,
    [["p", ALICE]],
    same,
    () => false,
  );
  assert.deepEqual(untrusted.mentionRefs, []);
  const renamed = buildEditMentionState(
    `@Old Scout (${BOB}) 12!`,
    [["mention", BOB]],
    undefined,
    () => true,
  );
  assert.deepEqual(renamed.mentionRefs, [
    { displayName: `Old Scout (${BOB}) 12`, pubkey: BOB, isAgent: true },
  ]);
});

test("an unbound qualified label cannot fall back to a shorter recipient", async () => {
  const { snapshotDraftMentionRefs } = await import("./draftMentionRefs.ts");
  const { extractMentionPubkeys } = await import("./extractMentionPubkeys.ts");
  const { buildMentionPattern } = await import(
    "../../../shared/lib/mentionPattern.ts"
  );
  const bindings = new Map([["Scout", ALICE]]);
  const text = `@Scout (${BOB}) hello`;
  assert.deepEqual(snapshotDraftMentionRefs(text, bindings, []), []);
  assert.deepEqual(
    extractMentionPubkeys({
      text,
      selectedMentions: bindings,
      memberCandidates: [],
    }),
    [],
  );
  assert.equal(buildMentionPattern(["Scout"]).test(text), false);
  assert.deepEqual(
    buildEditMentionState(
      text,
      [["p", ALICE]],
      { [ALICE]: { displayName: "Scout" } },
      () => false,
    ).mentionRefs,
    [],
  );
});

test("one-shot persona label iterators still shadow shorter selected recipients", async () => {
  const { snapshotDraftMentionRefs } = await import("./draftMentionRefs.ts");
  const { extractMentionPubkeys } = await import("./extractMentionPubkeys.ts");
  const bindings = new Map([["Scout", ALICE]]);
  const personas = new Map([["Scout Jones", "persona-id"]]);
  assert.deepEqual(
    snapshotDraftMentionRefs(
      "@Scout Jones",
      bindings,
      ["Scout"],
      [],
      personas.keys(),
    ),
    [],
  );
  assert.deepEqual(
    extractMentionPubkeys({
      text: "@Scout Jones",
      selectedMentions: bindings,
      selectedDisplayNames: personas.keys(),
      memberCandidates: [],
    }),
    [],
  );
});

test("qualified history consumes the entire multi-digit collision suffix", () => {
  for (const suffix of [2, 12, 22, 99, 222]) {
    const displayName = `Old Scout (${BOB}) ${suffix}`;
    const state = buildEditMentionState(
      `@${displayName}!`,
      [["mention", BOB]],
      undefined,
      () => true,
    );
    assert.deepEqual(state.mentionRefs, [
      { displayName, pubkey: BOB, isAgent: true },
    ]);
  }
});

test("authoritative edit snapshots exclude old p-tags and annotated tray metadata on reopen", () => {
  const target = buildEditMentionState(
    "@Bob edited",
    [
      ["p", ALICE],
      ["mention", ALICE, "agent-address"],
      ["mention", BOB],
      ["buzz:mention-snapshot"],
    ],
    { [ALICE]: { displayName: "Alice" }, [BOB]: { displayName: "Bob" } },
    () => false,
  );
  assert.deepEqual(target.mentionRefs, [
    { displayName: "Bob", pubkey: BOB, isAgent: false },
  ]);
  assert.deepEqual(target.unresolvedMentionPubkeys, []);
});
