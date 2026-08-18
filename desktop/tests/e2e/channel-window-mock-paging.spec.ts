import { expect, test } from "@playwright/test";

import { parseChannelWindowResponse } from "@/features/messages/lib/channelWindowResponse";
import type { RelayEvent } from "@/shared/api/types";

import { installMockBridge } from "../helpers/bridge";

// Focused mock-mode regression for the get_channel_window bridge handler.
//
// The relay-mode parity gate (parity-ancestor-island) never exercises the mock
// branch (identity present). A rows-only mock branch broke every ordinary
// mock-mode channel open, because parseChannelWindowResponse requires exactly
// one kind-39006 bounds event and throws otherwise. This proves the mock branch
// now returns a parseable page in both positions of a two-page walk, and that
// the page-1/page-2 union of rows has no duplication and no loss.
//
// Uses the empty `random` channel (no pre-seeds) so union math is exact.
const RANDOM_CHANNEL_ID = "9dae0116-799b-5071-a0a8-fdd30a91a35d";
const PAGE_CAP = 50; // getChannelWindowEvents default limitRows
const SEED_COUNT = 75; // > one page, so page-1 is full and page-2 holds the tail

async function invokeWindow(
  page: import("@playwright/test").Page,
  payload: Record<string, unknown>,
) {
  return page.evaluate(
    ([channelId, cursor, limitRows]) =>
      (window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__ as unknown as WindowInvoke)(
        "get_channel_window",
        { channelId, cursor, limitRows },
      ),
    [payload.channelId, payload.cursor, payload.limitRows] as const,
  );
}

type WindowInvoke = (
  command: string,
  payload?: Record<string, unknown>,
) => Promise<RelayEvent[]>;

test("mock-mode channel window pages parse with no dup or loss across page-1/page-2", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.waitForFunction(
    () =>
      typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function" &&
      typeof window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__ === "function",
  );

  // Seed SEED_COUNT top-level messages, strictly increasing created_at so their
  // relay order (created_at DESC, id ASC) is deterministic.
  await page.evaluate(
    ({ seedCount }) => {
      const base = 1_700_000_000;
      for (let index = 0; index < seedCount; index += 1) {
        window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
          channelName: "random",
          content: `paging ${index}`,
          createdAt: base + index,
        });
      }
    },
    { seedCount: SEED_COUNT },
  );

  // Page 1: head (no cursor).
  const rawPage1 = await invokeWindow(page, {
    channelId: RANDOM_CHANNEL_ID,
    cursor: null,
    limitRows: PAGE_CAP,
  });
  const page1 = parseChannelWindowResponse(rawPage1, RANDOM_CHANNEL_ID, null);
  expect(page1.rows.length).toBe(PAGE_CAP);
  expect(page1.hasMore).toBe(true);
  expect(page1.nextCursor).not.toBeNull();

  // Page 2: feed page-1's signed cursor back verbatim as the request cursor.
  const cursor = page1.nextCursor;
  if (!cursor) throw new Error("page-1 must expose a next cursor");
  const rawPage2 = await invokeWindow(page, {
    channelId: RANDOM_CHANNEL_ID,
    cursor: { created_at: cursor.createdAt, event_id: cursor.eventId },
    limitRows: PAGE_CAP,
  });
  const page2 = parseChannelWindowResponse(rawPage2, RANDOM_CHANNEL_ID, cursor);
  expect(page2.rows.length).toBe(SEED_COUNT - PAGE_CAP);
  expect(page2.hasMore).toBe(false);
  expect(page2.nextCursor).toBeNull();

  // Union: no duplication, no loss — exactly the SEED_COUNT distinct rows.
  const ids = [
    ...page1.rows.map((row) => row.event.id),
    ...page2.rows.map((row) => row.event.id),
  ];
  const unique = new Set(ids);
  expect(unique.size).toBe(SEED_COUNT); // no loss
  expect(ids.length).toBe(unique.size); // no duplication
});

test("mock-mode channel window includes summaries and the two-hop aux closure", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.waitForFunction(
    () =>
      typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function" &&
      typeof window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__ === "function",
  );

  const seeded = await page.evaluate(() => {
    const emit = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
    if (!emit) throw new Error("mock emitter is not installed");
    const root = emit({ channelName: "random", content: "window root" });
    const reply = emit({
      channelName: "random",
      content: "window reply",
      parentEventId: root.id,
    });
    const reaction = emit({
      channelName: "random",
      content: "🔥",
      kind: 7,
      extraTags: [["e", root.id]],
    });
    const edit = emit({
      channelName: "random",
      content: "edited window root",
      kind: 40003,
      extraTags: [["e", root.id]],
    });
    const rowDeletion = emit({
      channelName: "random",
      content: "",
      kind: 5,
      extraTags: [["e", root.id]],
    });
    const reactionDeletion = emit({
      channelName: "random",
      content: "",
      kind: 5,
      extraTags: [["e", reaction.id]],
    });
    return {
      rootId: root.id,
      replyId: reply.id,
      auxIds: [reaction.id, edit.id, rowDeletion.id, reactionDeletion.id],
    };
  });

  const raw = await invokeWindow(page, {
    channelId: RANDOM_CHANNEL_ID,
    cursor: null,
    limitRows: PAGE_CAP,
  });
  const parsed = parseChannelWindowResponse(raw, RANDOM_CHANNEL_ID, null);

  expect(parsed.rows.map((row) => row.event.id)).toEqual([seeded.rootId]);
  expect(parsed.aux.map((event) => event.id)).toEqual(
    expect.arrayContaining(seeded.auxIds),
  );
  expect(parsed.aux.map((event) => event.id)).not.toContain(seeded.replyId);
  expect(parsed.rows[0].thread).toMatchObject({
    replyCount: 1,
    descendantCount: 1,
  });
});
