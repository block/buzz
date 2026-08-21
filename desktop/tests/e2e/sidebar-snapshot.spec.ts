import { expect, test, type Page, bootstrapE2ePage } from "../helpers/test";

import { installMockBridge } from "../helpers/bridge";

const RELAY_URL = "ws://localhost:3000";
const OTHER_RELAY_URL = "ws://localhost:3001";
const OWNER_PUBKEY = "deadbeef".repeat(8);
const STALE_COMMUNITY_PUBKEY = "cafebabe".repeat(8);
const MATCHING_HASH = "mock-hash";
const READ_DELAY_MS = 600;
const SNAPSHOT_FRAME_DELAY_MS = 3_000;

function snapshotKey(relayUrl: string, ownerPubkey = OWNER_PUBKEY) {
  return `buzz-channels.v1:${relayUrl}:${ownerPubkey.toLowerCase()}`;
}

function makeSnapshotChannel(index: number, prefix = "snapshot") {
  return {
    id: `${prefix}-${index}`,
    name: `${prefix}-${String(index).padStart(2, "0")}`,
    channelType: "stream",
    visibility: "open",
    description: "",
    topic: null,
    purpose: null,
    memberCount: 1,
    memberPubkeys: [OWNER_PUBKEY],
    lastMessageAt: null,
    archivedAt: null,
    participants: [],
    participantPubkeys: [],
    isMember: true,
    ttlSeconds: null,
    ttlDeadline: null,
  };
}

const FULL_SNAPSHOT = Array.from({ length: 14 }, (_, index) =>
  makeSnapshotChannel(index),
);
const SNAPSHOT_DIAGNOSTIC_MARK = "buzz:sidebar:snapshot-diagnostic";
const FULL_SIDEBAR_PAINT_MARK = "buzz:sidebar:full-list-painted";
const BOOT_TO_FULL_SIDEBAR_MEASURE = "buzz:sidebar:boot-to-full-list-painted";

function snapshotIntegrity(
  ownerPubkey: string,
  hash: string,
  channels: ReturnType<typeof makeSnapshotChannel>[],
) {
  const value = JSON.stringify([ownerPubkey.toLowerCase(), hash, channels]);
  let result = 0x811c9dc5;
  for (let index = 0; index < value.length; index += 1) {
    result ^= value.charCodeAt(index);
    result = Math.imul(result, 0x01000193);
  }
  return (result >>> 0).toString(16).padStart(8, "0");
}

type SnapshotDiagnostics = {
  ageMs: number;
  channelCount: number;
  presence: "absent" | "invalid" | "present";
  serializedBytes: number;
};

async function getSnapshotDiagnostic(page: Page) {
  return page.evaluate((name) => {
    const entry = performance.getEntriesByName(name).at(-1) as
      | PerformanceMark
      | undefined;
    return entry?.detail as SnapshotDiagnostics | undefined;
  }, SNAPSHOT_DIAGNOSTIC_MARK);
}

async function getFullSidebarMeasure(page: Page) {
  return page.evaluate(
    ({ markName, measureName }) => ({
      markCount: performance.getEntriesByName(markName).length,
      measure: performance.getEntriesByName(measureName).at(-1)?.duration,
    }),
    {
      markName: FULL_SIDEBAR_PAINT_MARK,
      measureName: BOOT_TO_FULL_SIDEBAR_MEASURE,
    },
  );
}

async function seedSnapshot(
  page: Page,
  options: {
    channels?: ReturnType<typeof makeSnapshotChannel>[];
    hash: string;
    keyOwnerPubkey?: string;
    ownerPubkey?: string;
    relayUrl?: string;
  },
) {
  const channels = options.channels ?? FULL_SNAPSHOT;
  const ownerPubkey = options.ownerPubkey ?? OWNER_PUBKEY;
  await page.addInitScript(
    ({ channels, hash, integrity, key, ownerPubkey }) => {
      window.localStorage.setItem(
        key,
        JSON.stringify({
          version: 2,
          updatedAt: Date.now(),
          ownerPubkey,
          hash,
          channels,
          integrity,
        }),
      );
    },
    {
      channels,
      hash: options.hash,
      integrity: snapshotIntegrity(ownerPubkey, options.hash, channels),
      key: snapshotKey(
        options.relayUrl ?? RELAY_URL,
        options.keyOwnerPubkey ?? ownerPubkey,
      ),
      ownerPubkey,
    },
  );
}

async function seedCommunities(page: Page, communityPubkey = OWNER_PUBKEY) {
  await page.addInitScript(
    ({ communityPubkey, relayA, relayB }) => {
      const communities = [
        {
          id: "community-a",
          name: "Alpha",
          relayUrl: relayA,
          pubkey: communityPubkey,
          addedAt: "2026-01-01T00:00:00.000Z",
        },
        {
          id: "community-b",
          name: "Bravo",
          relayUrl: relayB,
          pubkey: communityPubkey,
          addedAt: "2026-01-02T00:00:00.000Z",
        },
      ];
      window.localStorage.setItem(
        "buzz-communities",
        JSON.stringify(communities),
      );
      window.localStorage.setItem(
        "buzz-active-community-id",
        communities[0].id,
      );
    },
    { communityPubkey, relayA: RELAY_URL, relayB: OTHER_RELAY_URL },
  );
}

async function getChannelsPayloads(page: Page) {
  return page.evaluate(() =>
    (window.__BUZZ_E2E_COMMAND_LOG__ ?? [])
      .filter((entry) => entry.command === "get_channels")
      .map((entry) => entry.payload as { knownHash?: unknown }),
  );
}

async function mutateDisplayedChannels(page: Page, channelName: string) {
  await page.evaluate((name) => {
    const queryClient = window.__BUZZ_E2E_QUERY_CLIENT__ as
      | {
          setQueryData: (
            queryKey: readonly string[],
            updater: (
              channels: ReturnType<typeof makeSnapshotChannel>[] | undefined,
            ) => ReturnType<typeof makeSnapshotChannel>[],
          ) => unknown;
        }
      | undefined;
    if (!queryClient) {
      throw new Error("E2E query client seam is not installed.");
    }
    queryClient.setQueryData(["channels"], (channels) => {
      const template = channels?.[0];
      if (!template) {
        throw new Error("Displayed channel cache is empty.");
      }
      return [...channels, { ...template, id: "optimistic-channel", name }];
    });
  }, channelName);
}

async function readPersistedSnapshot(page: Page) {
  return page.evaluate((key) => {
    const raw = window.localStorage.getItem(key);
    if (!raw) return null;
    const snapshot = JSON.parse(raw) as {
      channels: Array<{ name: string }>;
      hash: string;
    };
    return {
      channelNames: snapshot.channels.map((channel) => channel.name),
      hash: snapshot.hash,
    };
  }, snapshotKey(RELAY_URL));
}

async function trackSnapshotRows(page: Page) {
  await page.addInitScript(() => {
    const testWindow = window as Window & {
      __BUZZ_E2E_SNAPSHOT_ROWS_SEEN__?: string[];
    };
    const seen: string[] = [];
    testWindow.__BUZZ_E2E_SNAPSHOT_ROWS_SEEN__ = seen;
    const record = (node: Node) => {
      if (!(node instanceof Element)) return;
      const rows = node.matches('[data-channel-id^="snapshot-"]')
        ? [node]
        : Array.from(node.querySelectorAll('[data-channel-id^="snapshot-"]'));
      for (const row of rows) {
        const id = row.getAttribute("data-channel-id");
        if (id && !seen.includes(id)) seen.push(id);
      }
    };
    new MutationObserver((records) => {
      for (const mutation of records) {
        for (const node of mutation.addedNodes) record(node);
      }
    }).observe(document, { childList: true, subtree: true });
  });
}

async function getTrackedSnapshotRows(page: Page) {
  return page.evaluate(
    () =>
      (
        window as Window & {
          __BUZZ_E2E_SNAPSHOT_ROWS_SEEN__?: string[];
        }
      ).__BUZZ_E2E_SNAPSHOT_ROWS_SEEN__ ?? [],
  );
}

test("cold boot paints the complete snapshot, sends its hash, and revalidates", async ({
  page,
}) => {
  await seedSnapshot(page, { hash: MATCHING_HASH });
  await installMockBridge(page, {
    channelsReadDelayMs: SNAPSHOT_FRAME_DELAY_MS,
    honorChannelsKnownHash: true,
  });

  const navigationStartedAt = performance.now();
  await bootstrapE2ePage(page, "/");
  const rows = page.locator('[data-channel-id^="snapshot-"]');
  await expect(rows).toHaveCount(FULL_SNAPSHOT.length, { timeout: 2_000 });
  const snapshotPaintedAt = performance.now();
  expect(snapshotPaintedAt - navigationStartedAt).toBeLessThan(
    SNAPSHOT_FRAME_DELAY_MS,
  );
  await expect(page.getByTestId("sidebar-loading")).toHaveCount(0);
  await expect
    .poll(() => getChannelsPayloads(page))
    .toEqual([{ knownHash: MATCHING_HASH }]);
  await expect
    .poll(() => getSnapshotDiagnostic(page))
    .toMatchObject({
      channelCount: FULL_SNAPSHOT.length,
      presence: "present",
    });
  const diagnostics = await getSnapshotDiagnostic(page);
  expect(diagnostics?.serializedBytes).toBeGreaterThan(0);
  expect(diagnostics?.ageMs).toBeGreaterThanOrEqual(0);

  // The persisted data is deliberately stale: it paints immediately but still
  // validates. The delayed matching response leaves a deterministic snapshot
  // frame, then not-modified must not shrink the list.
  await expect(rows).toHaveCount(FULL_SNAPSHOT.length);
  await expect
    .poll(() => getFullSidebarMeasure(page))
    .toEqual({
      markCount: expect.any(Number),
      measure: expect.any(Number),
    });
  expect((await getFullSidebarMeasure(page)).markCount).toBeGreaterThan(0);

  const liveSettledAt = performance.now();
  console.info(
    `[sidebar-snapshot] firstPaint=${Math.round(snapshotPaintedAt - navigationStartedAt)}ms snapshotToLive=${Math.round(liveSettledAt - snapshotPaintedAt)}ms rows=${FULL_SNAPSHOT.length}`,
  );
});

test("matching not-modified preserves display mutations without persisting them", async ({
  page,
}) => {
  const optimisticName = "optimistic-local-channel";
  await seedSnapshot(page, { hash: MATCHING_HASH });
  await installMockBridge(page, {
    channelsReadDelayMs: READ_DELAY_MS,
    honorChannelsKnownHash: true,
  });
  await bootstrapE2ePage(page, "/");

  await expect(page.locator('[data-channel-id^="snapshot-"]')).toHaveCount(
    FULL_SNAPSHOT.length,
    { timeout: 500 },
  );
  await mutateDisplayedChannels(page, optimisticName);
  await expect(
    page.locator('[data-channel-id="optimistic-channel"]'),
  ).toBeVisible();

  await expect
    .poll(() => getChannelsPayloads(page))
    .toEqual([{ knownHash: MATCHING_HASH }]);
  await expect
    .poll(() => readPersistedSnapshot(page))
    .toEqual({
      channelNames: FULL_SNAPSHOT.map((channel) => channel.name),
      hash: MATCHING_HASH,
    });
  await expect(
    page.locator('[data-channel-id="optimistic-channel"]'),
  ).toBeVisible();
  expect((await readPersistedSnapshot(page))?.channelNames).not.toContain(
    optimisticName,
  );
});

test("first-ever boot without a snapshot sends null and shows loading", async ({
  page,
}) => {
  await installMockBridge(page, { channelsReadDelayMs: READ_DELAY_MS });
  await bootstrapE2ePage(page, "/");

  await expect(page.getByTestId("sidebar-loading")).toBeVisible({
    timeout: 500,
  });
  await expect(page.locator('[data-channel-id^="snapshot-"]')).toHaveCount(0);
  await expect
    .poll(() => getChannelsPayloads(page))
    .toEqual([{ knownHash: null }]);
  await expect
    .poll(() => getSnapshotDiagnostic(page))
    .toMatchObject({
      channelCount: 0,
      presence: "absent",
      serializedBytes: 0,
    });
  await expect(page.getByTestId("channel-general")).toBeVisible();
  await expect
    .poll(() => getFullSidebarMeasure(page))
    .toEqual({
      markCount: expect.any(Number),
      measure: expect.any(Number),
    });
  expect((await getFullSidebarMeasure(page)).markCount).toBeGreaterThan(0);
});

test("a different identity's snapshot is ignored", async ({ page }) => {
  await seedSnapshot(page, {
    hash: "foreign-hash",
    keyOwnerPubkey: OWNER_PUBKEY,
    ownerPubkey: STALE_COMMUNITY_PUBKEY,
  });
  await installMockBridge(page, { channelsReadDelayMs: READ_DELAY_MS });
  await bootstrapE2ePage(page, "/");

  await expect(page.getByTestId("sidebar-loading")).toBeVisible({
    timeout: 500,
  });
  await expect(page.locator('[data-channel-id^="snapshot-"]')).toHaveCount(0);
  await expect
    .poll(() => getChannelsPayloads(page))
    .toEqual([{ knownHash: null }]);
  await expect
    .poll(() => getSnapshotDiagnostic(page))
    .toMatchObject({
      channelCount: 0,
      presence: "invalid",
    });
  await expect(page.getByTestId("channel-general")).toBeVisible();
});

test("display-only community pubkey cannot expose another identity's snapshot", async ({
  page,
}) => {
  await seedSnapshot(page, {
    hash: "stale-community-identity-hash",
    ownerPubkey: STALE_COMMUNITY_PUBKEY,
  });
  await trackSnapshotRows(page);
  await installMockBridge(
    page,
    { channelsReadDelayMs: READ_DELAY_MS },
    { skipCommunitySeed: true },
  );
  await seedCommunities(page, STALE_COMMUNITY_PUBKEY);
  await bootstrapE2ePage(page, "/");

  await expect(page.getByTestId("channel-general")).toBeVisible();
  expect(await getTrackedSnapshotRows(page)).toEqual([]);
  const payloads = await getChannelsPayloads(page);
  expect(payloads).toEqual([{ knownHash: null }]);
  expect(payloads).not.toContainEqual({
    knownHash: "stale-community-identity-hash",
  });
});

test("partial hash/list write fails toward a full fetch", async ({ page }) => {
  await seedSnapshot(page, { hash: MATCHING_HASH });
  await page.addInitScript((key) => {
    const raw = window.localStorage.getItem(key);
    if (!raw) return;
    const snapshot = JSON.parse(raw) as { hash: string };
    window.localStorage.setItem(
      key,
      JSON.stringify({ ...snapshot, hash: "newer-partial-hash" }),
    );
  }, snapshotKey(RELAY_URL));
  await installMockBridge(page, {
    channelsReadDelayMs: READ_DELAY_MS,
    honorChannelsKnownHash: true,
  });
  await bootstrapE2ePage(page, "/");

  await expect(page.getByTestId("sidebar-loading")).toBeVisible({
    timeout: 500,
  });
  await expect(page.locator('[data-channel-id^="snapshot-"]')).toHaveCount(0);
  await expect
    .poll(() => getChannelsPayloads(page))
    .toEqual([{ knownHash: null }]);
  await expect
    .poll(() => getSnapshotDiagnostic(page))
    .toMatchObject({
      channelCount: 0,
      presence: "invalid",
    });
  await expect(page.getByTestId("channel-general")).toBeVisible();
  await expect(page.locator('[data-channel-id^="snapshot-"]')).toHaveCount(0);
});

test("unexpected not-modified response retries without a hash", async ({
  page,
}) => {
  await installMockBridge(page, {
    channelsNotModifiedResponses: 1,
  });
  await bootstrapE2ePage(page, "/");

  await expect(page.getByTestId("channel-general")).toBeVisible();
  await expect
    .poll(() => getChannelsPayloads(page))
    .toEqual([{ knownHash: null }, { knownHash: null }]);
});

test("mismatched not-modified hash falls back to a full list", async ({
  page,
}) => {
  await seedSnapshot(page, { hash: "persisted-stale-hash" });
  await installMockBridge(page, {
    channelsReadDelayMs: READ_DELAY_MS,
    channelsNotModifiedResponses: 1,
  });
  await bootstrapE2ePage(page, "/");

  const snapshotRows = page.locator('[data-channel-id^="snapshot-"]');
  await expect(snapshotRows).toHaveCount(FULL_SNAPSHOT.length, {
    timeout: 500,
  });
  await expect
    .poll(() => getChannelsPayloads(page))
    .toEqual([{ knownHash: "persisted-stale-hash" }, { knownHash: null }]);

  // The stale snapshot may provide the immediate boot frame, but an impossible
  // not-modified hash/list pairing must never survive as the settled result.
  await expect(page.getByTestId("channel-general")).toBeVisible();
  await expect(snapshotRows).toHaveCount(0);
  await expect
    .poll(() =>
      page.evaluate((key) => {
        const raw = window.localStorage.getItem(key);
        if (!raw) return null;
        const snapshot = JSON.parse(raw) as {
          channels: Array<{ name: string }>;
          hash: string;
        };
        return {
          hash: snapshot.hash,
          hasGeneral: snapshot.channels.some(
            (channel) => channel.name === "general",
          ),
          hasSnapshotRow: snapshot.channels.some((channel) =>
            channel.name.startsWith("snapshot-"),
          ),
        };
      }, snapshotKey(RELAY_URL)),
    )
    .toEqual({ hash: MATCHING_HASH, hasGeneral: true, hasSnapshotRow: false });
});

test("hash mismatch replaces the snapshot with the full live list", async ({
  page,
}) => {
  await seedSnapshot(page, { hash: "stale-hash" });
  await installMockBridge(page, {
    channelsReadDelayMs: READ_DELAY_MS,
    honorChannelsKnownHash: true,
  });
  await bootstrapE2ePage(page, "/");

  await expect(page.locator('[data-channel-id^="snapshot-"]')).toHaveCount(
    FULL_SNAPSHOT.length,
    { timeout: 500 },
  );
  await expect
    .poll(() => getChannelsPayloads(page))
    .toEqual([{ knownHash: "stale-hash" }]);
  await expect(page.getByTestId("channel-general")).toBeVisible();
  await expect(page.locator('[data-channel-id^="snapshot-"]')).toHaveCount(0);
  await expect
    .poll(() =>
      page.evaluate((key) => {
        const raw = window.localStorage.getItem(key);
        if (!raw) return null;
        const snapshot = JSON.parse(raw) as {
          channels: Array<{ name: string }>;
          hash: string;
        };
        return {
          hash: snapshot.hash,
          hasGeneral: snapshot.channels.some(
            (channel) => channel.name === "general",
          ),
          hasSnapshotRow: snapshot.channels.some((channel) =>
            channel.name.startsWith("snapshot-"),
          ),
        };
      }, snapshotKey(RELAY_URL)),
    )
    .toEqual({ hash: MATCHING_HASH, hasGeneral: true, hasSnapshotRow: false });
});

test("community switch validates a stale relay snapshot and replaces it", async ({
  page,
}) => {
  const switchedSnapshot = Array.from({ length: 6 }, (_, index) =>
    makeSnapshotChannel(index, "switched"),
  );
  await seedSnapshot(page, {
    channels: switchedSnapshot,
    hash: "stale-community-hash",
    relayUrl: OTHER_RELAY_URL,
  });
  await installMockBridge(
    page,
    {
      channelsReadDelayMs: READ_DELAY_MS,
      honorChannelsKnownHash: true,
    },
    { skipCommunitySeed: true },
  );
  await seedCommunities(page);
  await bootstrapE2ePage(page, "/");
  await expect(page.getByTestId("app-sidebar")).toBeVisible();

  const payloadCountBeforeSwitch = (await getChannelsPayloads(page)).length;
  await page.getByTestId("community-rail-button-community-b").click();
  const switchedRows = page.locator('[data-channel-id^="switched-"]');
  await expect(switchedRows).toHaveCount(switchedSnapshot.length, {
    timeout: 500,
  });
  await expect
    .poll(async () =>
      (await getChannelsPayloads(page)).slice(payloadCountBeforeSwitch).at(0),
    )
    .toEqual({ knownHash: "stale-community-hash" });

  await expect(page.getByTestId("channel-general")).toBeVisible();
  await expect(switchedRows).toHaveCount(0);
  await expect
    .poll(() =>
      page.evaluate((key) => {
        const raw = window.localStorage.getItem(key);
        if (!raw) return null;
        const snapshot = JSON.parse(raw) as {
          channels: Array<{ name: string }>;
          hash: string;
        };
        return {
          hash: snapshot.hash,
          hasGeneral: snapshot.channels.some(
            (channel) => channel.name === "general",
          ),
        };
      }, snapshotKey(OTHER_RELAY_URL)),
    )
    .toEqual({ hash: MATCHING_HASH, hasGeneral: true });
});
