import assert from "node:assert/strict";
import test from "node:test";
import {
  buildEditMentionState,
  snapshotDraftMentionRefs,
} from "./draftMentionRefs.ts";
import { extractMentionPubkeys } from "./extractMentionPubkeys.ts";
import { getSendToChannelSemantics } from "./sendToChannelSemantics.ts";
import { getVisibleAgentAddressPubkeys } from "./getVisibleAgentAddressPubkeys.ts";
import { resolveMentionProps } from "../../../shared/lib/resolveMentionNames.ts";
import { submitMessageEdit } from "../ui/submitMessageEdit.ts";

const A = "a".repeat(64),
  B = "b".repeat(64),
  C = "c".repeat(64);
const originalRef = { displayName: "Scout", pubkey: A, isAgent: true };

for (const replacement of ["member", "persona", "selected", "ambiguous"]) {
  test(`edit fallback competes with longer ${replacement} before save and restore`, async () => {
    const members =
      replacement === "member" || replacement === "ambiguous"
        ? [
            { displayName: "Scout Jones", pubkey: B, isMember: true },
            ...(replacement === "ambiguous"
              ? [{ displayName: "Scout Jones", pubkey: C, isMember: true }]
              : []),
          ]
        : [];
    const bindings = new Map([
      ["Scout", A],
      ...(replacement === "selected" ? [["Scout Jones", B]] : []),
    ]);
    const personas = replacement === "persona" ? ["Scout Jones"] : [];
    let saved,
      restored,
      cleared = false,
      error;
    const options = {
      content: "@Scout Jones hello",
      originalContent: "@Scout hello",
      editTarget: { mentionRefs: [originalRef], unresolvedMentionPubkeys: [] },
      editTargetId: "dummy-event",
      ownerPubkey: C,
      customEmoji: [],
      pendingImeta: [],
      queuedAttachments: [],
      spoileredAttachmentUrls: new Set(),
      getMentionRefs: (text, fallback) =>
        snapshotDraftMentionRefs(
          text,
          bindings,
          ["Scout"],
          members,
          personas,
          fallback,
        ),
      extractMentionPubkeys: (text) =>
        extractMentionPubkeys({
          text,
          selectedMentions: bindings,
          selectedDisplayNames: personas,
          memberCandidates: members,
        }),
      clearComposer() {
        cleared = true;
      },
      restoreComposer() {},
      restoreMentionRefs(refs) {
        restored = refs;
      },
      revalidateMentionPubkeys: async (keys) => [...keys],
      setDeferredUploadPending() {},
      shouldRestoreComposer: () => true,
      setUploadError(message) {
        error = message;
      },
      save: async (content, tags, notifying) => {
        saved = { content, tags, notifying };
      },
    };
    await submitMessageEdit(options);
    if (replacement === "ambiguous") {
      assert.match(error, /ambiguous/);
      assert.equal(cleared, false);
      assert.equal(saved, undefined);
      return;
    }
    assert.deepEqual(
      saved.tags ?? [],
      replacement === "selected" || replacement === "member"
        ? [["mention", B]]
        : [],
    );
    assert.deepEqual(saved.notifying, replacement === "persona" ? [] : [B]);
    await submitMessageEdit({
      ...options,
      save: async () => {
        throw new Error("offline");
      },
    });
    assert.deepEqual(
      restored.map((ref) => ref.pubkey),
      replacement === "selected" ? [B] : [],
    );
  });
}

test("fallback keeps missing-profile refs but current same-name selection wins", () => {
  assert.deepEqual(
    snapshotDraftMentionRefs(
      "@Scout hello",
      new Map(),
      [],
      [],
      [],
      [originalRef],
    ),
    [originalRef],
  );
  assert.deepEqual(
    snapshotDraftMentionRefs(
      "@Scout hello",
      new Map([["Scout", B]]),
      [],
      [],
      [],
      [originalRef],
    ),
    [{ displayName: "Scout", pubkey: B, isAgent: false }],
  );
  assert.deepEqual(
    snapshotDraftMentionRefs(
      "@Scout Jones hello",
      new Map(),
      [],
      [],
      ["Scout"],
      [originalRef],
    ),
    [],
  );
});

for (const prefix of ["Scout", "Scout @Jones"]) {
  test(`ambiguous longer alias blocks ${prefix} in hydration and sibling fallbacks`, () => {
    const longer = `${prefix} Jones`;
    const profiles = {
      [A]: { displayName: prefix },
      [B]: { displayName: longer },
      [C]: { displayName: longer },
    };
    for (const keys of [
      [A, B, C],
      [C, B, A],
    ]) {
      const tags = keys.map((key) => ["p", key]);
      const text = `@${longer} hello`;
      const props = resolveMentionProps(tags, profiles, text);
      assert.deepEqual(
        buildEditMentionState(text, tags, profiles, () => true).mentionRefs,
        [],
      );
      assert.deepEqual(
        new Set(
          buildEditMentionState(text, tags, profiles, () => true)
            .unresolvedMentionPubkeys,
        ),
        new Set([A, B, C]),
      );
      assert.deepEqual(
        getVisibleAgentAddressPubkeys(
          text,
          [A],
          props.mentionPubkeysByName,
          props.mentionNames,
        ),
        [A],
      );
      assert.deepEqual(
        getSendToChannelSemantics(
          { body: text, edited: true, pubkey: "d".repeat(64), tags },
          profiles,
        ).mentionPubkeys,
        [],
      );
      // Blocking one occurrence must not erase a separate valid shorter one.
      const mixed = `${text} @${prefix}!`;
      assert.deepEqual(
        buildEditMentionState(mixed, tags, profiles, () => true).mentionRefs,
        [{ displayName: prefix, pubkey: A, isAgent: true }],
      );
    }
  });
}

test("a shadowed or ambiguous qualified alias cannot disambiguate another occurrence", () => {
  for (const suffix of [" Jones", ""]) {
    const qualified = `Scout (${B})`;
    const longer = `${qualified}${suffix}`;
    const content = `@Scout and @${longer} hello`;
    const state = buildEditMentionState(
      content,
      [
        ["p", A],
        ["p", B],
        ["p", C],
      ],
      {
        [A]: { displayName: "Scout" },
        [B]: { displayName: "Scout" },
        [C]: { displayName: longer },
      },
      () => true,
    );
    assert.equal(
      state.mentionRefs.some((ref) => ref.pubkey === A),
      false,
    );
    assert.deepEqual(
      state.mentionRefs.map((ref) => ref.pubkey),
      suffix ? [C] : [],
    );
  }
});
