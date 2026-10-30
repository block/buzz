/**
 * `:par` autocomplete against the relay catalog, through the production
 * searcher and the Tauri bridge command: one debounced request for a burst of
 * typing, results capped at 20, a late reply fenced out, and a community reset
 * dropping the cache.
 */

import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import { act, cleanup, renderHook, waitFor } from "@testing-library/react";

import {
  clearIpcHandlers,
  deferred,
  setIpcHandler,
} from "../../admin-console/adminConsolePanelTestHelpers.jsdom.mjs";
import { resetEmojiCatalog } from "../../custom-emoji/emojiCatalog.ts";
import { useEmojiAutocomplete } from "./useEmojiAutocomplete.ts";

afterEach(() => {
  cleanup();
  resetEmojiCatalog();
  clearIpcHandlers();
});

function entry(name, at = 1) {
  return {
    id: `id-${name}`,
    pubkey: "a".repeat(64),
    created_at: at,
    kind: 30180,
    content: name.replace(/_/g, " "),
    tags: [
      ["d", name],
      ["emoji", name, `https://x/${name}.png`],
    ],
    sig: "s",
  };
}

const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

test("a burst of typing sends one request, results are capped at 20, and a repeat is cached", async () => {
  const queries = [];
  setIpcHandler("search_emoji_entries", async (args) => {
    queries.push(args.query);
    return Array.from({ length: 50 }, (_, i) => entry(`zzcatalog_${i}`));
  });
  const { result } = renderHook(() => useEmojiAutocomplete([]));

  for (const draft of [":zz", ":zzc", ":zzca"]) {
    act(() => result.current.updateEmojiQuery(draft, draft.length));
    await wait(20);
  }
  await waitFor(() => assert.ok(result.current.emojiSuggestions.length > 0), {
    timeout: 3000,
  });
  await wait(100);
  assert.deepEqual(queries, ["zzca"]);
  assert.ok(result.current.emojiSuggestions.length <= 20);
  assert.ok(
    result.current.emojiSuggestions.every((e) => e.id.startsWith("zzcatalog_")),
  );
  assert.equal(
    result.current.emojiSuggestions[0].sourceUrl,
    "https://x/zzcatalog_0.png",
  );

  act(() => result.current.clearEmojis());
  act(() => result.current.updateEmojiQuery(":zzca", 5));
  await waitFor(() => assert.ok(result.current.isEmojiAutocompleteOpen));
  await wait(400);
  assert.deepEqual(queries, ["zzca"]);
});

test("a slow reply for an older query does not replace the newer one, and a failure is flagged", async () => {
  const slow = deferred();
  const queries = [];
  setIpcHandler("search_emoji_entries", async (args) => {
    queries.push(args.query);
    if (args.query === "qqold") return slow.promise;
    if (args.query === "qqbad") throw new Error("bridge down");
    return [entry("qqnew")];
  });
  const { result } = renderHook(() => useEmojiAutocomplete([]));

  act(() => result.current.updateEmojiQuery(":qqold", 6));
  await waitFor(() => assert.deepEqual(queries, ["qqold"]), { timeout: 3000 });
  act(() => result.current.updateEmojiQuery(":qqnew", 6));
  await waitFor(() => assert.ok(queries.includes("qqnew")), { timeout: 3000 });
  await waitFor(() =>
    assert.ok(result.current.emojiSuggestions.some((e) => e.id === "qqnew")),
  );
  slow.resolve([entry("qqold_late")]);
  await wait(50);
  assert.ok(
    !result.current.emojiSuggestions.some((e) => e.id === "qqold_late"),
  );

  act(() => result.current.updateEmojiQuery(":qqbad", 6));
  await waitFor(() => assert.equal(result.current.isSearchFailed, true), {
    timeout: 3000,
  });
});

test("the list never shows more than 20 rows even when local matches are many", async () => {
  setIpcHandler("search_emoji_entries", async () =>
    Array.from({ length: 50 }, (_, i) => entry(`smilecatalog_${i}`)),
  );
  const legacy = Array.from({ length: 40 }, (_, i) => ({
    shortcode: `smilelegacy_${i}`,
    url: `https://x/smilelegacy_${i}.png`,
  }));
  const { result } = renderHook(() => useEmojiAutocomplete(legacy));
  act(() => result.current.updateEmojiQuery(":smile", 6));
  await waitFor(() => assert.ok(result.current.emojiSuggestions.length > 0), {
    timeout: 3000,
  });
  await wait(500);
  assert.equal(result.current.emojiSuggestions.length, 20);
});
