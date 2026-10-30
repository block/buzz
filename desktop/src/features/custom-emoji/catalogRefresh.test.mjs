import assert from "node:assert/strict";
import { test } from "node:test";

import { QueryClient, QueryObserver } from "@tanstack/react-query";

import { relayClient } from "../../shared/api/relayClient.ts";
import {
  COMMUNITY_EMOJI_ENTRY_CAP,
  COMMUNITY_EMOJI_SET_LIMIT,
} from "../../shared/api/customEmoji.ts";
import {
  customEmojiCatalogReportQueryKey,
  customEmojiQueryFn,
  customEmojiQueryKey,
} from "./hooks.ts";

function setEvent(tags, createdAt = 1) {
  return {
    id: "x",
    pubkey: "p",
    created_at: createdAt,
    kind: 30030,
    tags: [["d", "buzz:custom-emoji"], ...tags],
    content: "",
    sig: "s",
  };
}

test("a failed refresh keeps the last good palette and is not an empty success", async (t) => {
  t.mock.method(console, "warn", () => {});
  const responses = [
    [setEvent([["emoji", "old", "https://r/old.png"]])],
    new Error("relay down"),
  ];
  t.mock.method(relayClient, "fetchEvents", async () => {
    const next = responses.shift();
    if (next instanceof Error) throw next;
    return next;
  });
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const observer = new QueryObserver(queryClient, {
    queryKey: customEmojiQueryKey,
    queryFn: customEmojiQueryFn(queryClient),
  });
  const unsubscribe = observer.subscribe(() => {});
  await observer.refetch();
  assert.deepEqual(observer.getCurrentResult().data, [
    { shortcode: "old", url: "https://r/old.png" },
  ]);

  await observer.refetch();
  const result = observer.getCurrentResult();
  assert.equal(result.isRefetchError, true);
  assert.deepEqual(result.data, [
    { shortcode: "old", url: "https://r/old.png" },
  ]);
  unsubscribe();
});

test("the query publishes the catalog report for the hook to read", async (t) => {
  t.mock.method(console, "warn", () => {});
  const sets = Array.from({ length: COMMUNITY_EMOJI_SET_LIMIT - 1 }, (_, i) =>
    setEvent([["emoji", `e${i}`, "https://r/x.png"]], i),
  );
  sets.push(
    setEvent(
      Array.from({ length: COMMUNITY_EMOJI_ENTRY_CAP }, (_, i) => [
        "emoji",
        `z${i}`,
        "https://r/x.png",
      ]),
    ),
  );
  t.mock.method(relayClient, "fetchEvents", async () => sets);
  const queryClient = new QueryClient();
  const emoji = await queryClient.fetchQuery({
    queryKey: customEmojiQueryKey,
    queryFn: customEmojiQueryFn(queryClient),
  });
  assert.equal(emoji.length, COMMUNITY_EMOJI_ENTRY_CAP);
  const report = queryClient.getQueryData(customEmojiCatalogReportQueryKey);
  assert.equal(report.setLimitReached, true);
  assert.equal(report.droppedEntries, COMMUNITY_EMOJI_SET_LIMIT - 1);
});
