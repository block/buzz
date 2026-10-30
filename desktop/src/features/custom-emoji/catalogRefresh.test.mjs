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

import {
  customEmojiCatalogNotice,
  customEmojiCatalogStatus,
} from "./catalogStatus.ts";

async function observe(t, responses) {
  t.mock.method(console, "warn", () => {});
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
  t.after(unsubscribe);
  const status = () =>
    customEmojiCatalogStatus(
      observer.getCurrentResult(),
      queryClient.getQueryData(customEmojiCatalogReportQueryKey),
    );
  return { observer, queryClient, status };
}

test("status flags a failed first load, not just a failed refresh", async (t) => {
  const { observer, status } = await observe(t, [
    new Error("timeout"),
    [setEvent([["emoji", "a", "https://r/a.png"]])],
    new Error("down"),
  ]);
  await observer.refetch();
  assert.deepEqual(
    { failed: status().refreshFailed, has: status().hasCatalog },
    { failed: true, has: false },
  );
  assert.equal(
    customEmojiCatalogNotice(status()),
    "Could not load community emoji.",
  );

  await observer.refetch();
  assert.equal(status().refreshFailed, false);
  assert.equal(customEmojiCatalogNotice(status()), null);

  await observer.refetch();
  assert.equal(status().refreshFailed, true);
  assert.equal(status().hasCatalog, true);
  assert.match(customEmojiCatalogNotice(status()), /last loaded list/);
});

test("the catalog report is never garbage-collected", async (t) => {
  const { observer, queryClient } = await observe(t, [
    [setEvent([["emoji", "a", "https://r/a.png"]])],
  ]);
  await observer.refetch();
  const query = queryClient
    .getQueryCache()
    .find({ queryKey: customEmojiCatalogReportQueryKey });
  assert.equal(query.gcTime, Number.POSITIVE_INFINITY);
});

test("notice text covers every status branch", () => {
  const base = {
    refreshFailed: false,
    hasCatalog: true,
    setLimitReached: false,
    droppedEntries: 0,
  };
  assert.equal(customEmojiCatalogNotice(base), null);
  assert.match(
    customEmojiCatalogNotice({ ...base, droppedEntries: 7 }),
    /7 emoji are hidden/,
  );
  assert.match(
    customEmojiCatalogNotice({ ...base, setLimitReached: true }),
    /incomplete/,
  );
  assert.match(
    customEmojiCatalogNotice({
      ...base,
      refreshFailed: true,
      droppedEntries: 7,
    }),
    /Could not refresh/,
  );
});
