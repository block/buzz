import {
  expect,
  test,
  type Page,
  type Route,
  bootstrapE2ePage,
} from "../helpers/test";
import { hexToBytes } from "@noble/hashes/utils.js";
import { finalizeEvent, type VerifiedEvent } from "nostr-tools/pure";

import { installRelayBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { assertRelaySeeded } from "../helpers/seed";

const RELAY_HTTP = process.env.BUZZ_E2E_RELAY_URL ?? "http://localhost:3000";
const DM_ID = "5a9c064e-0411-5242-ae6b-0363ba99b8e6";

async function publishAliceDm(
  content: string,
  createdAt: number,
): Promise<VerifiedEvent> {
  const event = finalizeEvent(
    {
      kind: 9,
      content,
      created_at: createdAt,
      tags: [
        ["h", DM_ID],
        ["p", TEST_IDENTITIES.tyler.pubkey],
      ],
    },
    hexToBytes(TEST_IDENTITIES.alice.privateKey),
  );
  const response = await fetch(`${RELAY_HTTP}/events`, {
    method: "POST",
    headers: { "Content-Type": "application/json", "X-Pubkey": event.pubkey },
    body: JSON.stringify(event),
  });
  if (!response.ok) {
    throw new Error(
      `POST /events failed (${response.status}): ${await response.text()}`,
    );
  }
  return event;
}

async function timelineIds(page: Page) {
  return page.getByTestId("message-timeline").evaluate((element) =>
    Array.from(element.querySelectorAll<HTMLElement>("[data-message-id]"))
      .map((row) => row.dataset.messageId)
      .filter((id): id is string => Boolean(id)),
  );
}

async function cachedMessageIds(page: Page) {
  return page.evaluate((channelId) => {
    const client = window.__BUZZ_E2E_QUERY_CLIENT__ as unknown as {
      getQueryData: (
        key: readonly unknown[],
      ) => Array<{ id: string }> | undefined;
    };
    if (!client) throw new Error("E2E query client is unavailable.");
    return (client.getQueryData(["channel-messages", channelId]) ?? []).map(
      (event) => event.id,
    );
  }, DM_ID);
}

function isDmWindowQuery(route: Route) {
  const request = route.request();
  if (request.method() !== "POST" || !request.url().endsWith("/query")) {
    return false;
  }
  try {
    const body = request.postDataJSON() as Array<Record<string, unknown>>;
    return body.some(
      (filter) =>
        Array.isArray(filter["#h"]) &&
        filter["#h"].includes(DM_ID) &&
        filter.top_level === true &&
        filter.include_summaries === true &&
        filter.include_aux === true,
    );
  } catch {
    return false;
  }
}

async function dmLiveRequestCount(page: Page) {
  return page.evaluate((channelId) => {
    let count = 0;
    for (const entry of window.__BUZZ_E2E_COMMAND_LOG__ ?? []) {
      if (entry.command !== "plugin:websocket|send") continue;
      const data = (
        entry.payload as { message?: { data?: string } } | undefined
      )?.message?.data;
      if (!data) continue;
      try {
        const frame = JSON.parse(data) as [
          string,
          string,
          ...Record<string, unknown>[],
        ];
        if (
          frame[0] === "REQ" &&
          frame
            .slice(2)
            .some(
              (filter) =>
                Array.isArray(filter["#h"]) &&
                filter["#h"].includes(channelId) &&
                typeof filter.since === "number",
            )
        ) {
          count += 1;
        }
      } catch {
        // Ignore non-JSON websocket payloads.
      }
    }
    return count;
  }, DM_ID);
}

test.beforeAll(async () => {
  test.setTimeout(90_000);
  await assertRelaySeeded();
});

test("existing DM history remains when the first live DM reaches a pageless window", async ({
  page,
}) => {
  const baseSecond = Math.floor(Date.now() / 1000) - 10;
  const history = await Promise.all(
    [0, 1, 2].map((index) =>
      publishAliceDm(`dm history ${index}`, baseSecond + index),
    ),
  );
  const historyIds = history.map((event) => event.id);

  await installRelayBridge(page, "tyler");
  await bootstrapE2ePage(page, "/");
  await page.getByTestId("channel-alice-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("alice-tyler");
  await expect.poll(() => timelineIds(page)).toEqual(historyIds);

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect.poll(() => cachedMessageIds(page)).toEqual(historyIds);
  await page.evaluate((channelId) => {
    const client = window.__BUZZ_E2E_QUERY_CLIENT__ as unknown as {
      removeQueries: (filters: {
        queryKey: readonly unknown[];
        exact: boolean;
      }) => void;
    };
    if (!client) throw new Error("E2E query client is unavailable.");
    client.removeQueries({
      queryKey: ["channel-window", channelId],
      exact: true,
    });
  }, DM_ID);

  let heldDmWindowQuery = false;
  let releaseHeldQuery: (() => void) | undefined;
  let finishHeldQuery: (() => void) | undefined;
  const heldQueryReleased = new Promise<void>((resolve) => {
    releaseHeldQuery = resolve;
  });
  const heldQueryFinished = new Promise<void>((resolve) => {
    finishHeldQuery = resolve;
  });
  const holdDmWindowQuery = async (route: Route) => {
    if (!isDmWindowQuery(route)) {
      await route.continue();
      return;
    }
    heldDmWindowQuery = true;
    await heldQueryReleased;
    await route.continue();
    finishHeldQuery?.();
  };
  await page.route("**/query", holdDmWindowQuery);

  try {
    const priorLiveRequests = await dmLiveRequestCount(page);
    await page.getByTestId("channel-alice-tyler").click();
    await expect(page.getByTestId("chat-title")).toHaveText("alice-tyler");
    await expect
      .poll(() => dmLiveRequestCount(page))
      .toBeGreaterThan(priorLiveRequests);

    const live = await publishAliceDm(
      "dm live arrival",
      Math.floor(Date.now() / 1000),
    );
    const expectedIds = [...historyIds, live.id];

    await expect.poll(() => timelineIds(page)).toEqual(expectedIds);
    await expect.poll(() => cachedMessageIds(page)).toEqual(expectedIds);
    expect(new Set(expectedIds).size).toBe(expectedIds.length);
    await expect(page.getByTestId("channel-alice-tyler")).toBeVisible();
    await expect(page.getByTestId("message-input")).toBeVisible();
    await expect(page.getByTestId("message-input")).toBeEnabled();
  } finally {
    releaseHeldQuery?.();
    if (heldDmWindowQuery) await heldQueryFinished;
    await page.unroute("**/query", holdDmWindowQuery);
  }
});
