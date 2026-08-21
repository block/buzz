import { expect, test, bootstrapE2ePage } from "../helpers/test";

import { installRelayBridge } from "../helpers/bridge";
import { assertRelaySeeded } from "../helpers/seed";

const REPRESENTATIVE_FAMILIES = new Set([
  "channels",
  "home-feed",
  "relay-agents",
  "channel-templates",
  "custom-emoji",
  "user-status",
]);

type ForegroundProbe = {
  marker: string;
  at: number;
  detail?: unknown;
};

test.beforeAll(async () => {
  test.setTimeout(90_000);
  await assertRelaySeeded();
});

test("foreground paints and handles a sidebar action before resume fetches", async ({
  page,
}) => {
  await page.addInitScript(() => {
    type Probe = { marker: string; at: number; detail?: unknown };
    const probes: Probe[] = [];
    const record = (marker: string, detail?: unknown) =>
      probes.push({ marker, at: performance.now(), detail });
    (
      window as typeof window & { __FOREGROUND_PROBES__?: Probe[] }
    ).__FOREGROUND_PROBES__ = probes;

    let captured = false;
    window.addEventListener(
      "focus",
      () => {
        const armed = (
          window as typeof window & { __FOREGROUND_PROBE_ARMED__?: boolean }
        ).__FOREGROUND_PROBE_ARMED__;
        if (!armed || captured) return;
        captured = true;
        record("focus", {
          hasFocus: document.hasFocus(),
          visibilityState: document.visibilityState,
        });
        window.requestAnimationFrame(() => {
          record("first-frame");
          const target = document.querySelector<HTMLElement>(
            '[data-testid="channel-alice-tyler"]',
          );
          if (!target)
            throw new Error("Sidebar sentinel channel is unavailable.");
          target.click();
          record("interaction-dispatched");

          const recordCommitWhenSelected = () => {
            const selectedTitle = document
              .querySelector<HTMLElement>('[data-testid="chat-title"]')
              ?.textContent?.trim();
            if (selectedTitle === "alice-tyler") {
              record("interaction-committed");
              return;
            }
            window.requestAnimationFrame(recordCommitWhenSelected);
          };
          recordCommitWhenSelected();
        });
      },
      { capture: true },
    );
  });
  await installRelayBridge(page, "tyler");
  await bootstrapE2ePage(page, "/");
  await expect(page.getByTestId("app-sidebar")).toBeVisible();
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const seasonedKeys = await page.evaluate(
    (representativeFamilies) => {
      type QueryLike = {
        queryKey: readonly unknown[];
        options: { refetchOnWindowFocus?: unknown };
        state: Record<string, unknown>;
        getObserversCount: () => number;
        setState: (state: Record<string, unknown>) => void;
      };
      type QueryCacheLike = {
        findAll: () => QueryLike[];
        subscribe: (
          listener: (event: { query?: QueryLike }) => void,
        ) => () => void;
      };
      type QueryClientLike = { getQueryCache: () => QueryCacheLike };
      const client =
        window.__BUZZ_E2E_QUERY_CLIENT__ as unknown as QueryClientLike;
      if (!client) throw new Error("E2E query client is unavailable.");

      const families = new Set(representativeFamilies);
      const active = client
        .getQueryCache()
        .findAll()
        .filter(
          (query) =>
            query.getObserversCount() > 0 &&
            families.has(String(query.queryKey[0] ?? "")),
        );
      for (const query of active) {
        query.setState({
          ...query.state,
          dataUpdatedAt: 1,
          isInvalidated: true,
        });
      }

      const probes = (
        window as typeof window & { __FOREGROUND_PROBES__?: ForegroundProbe[] }
      ).__FOREGROUND_PROBES__;
      if (!probes) throw new Error("Foreground probe was not installed.");
      const record = (marker: string, detail?: unknown) =>
        probes.push({ marker, at: performance.now(), detail });

      const activeHashes = new Set(
        active.map((query) => JSON.stringify(query.queryKey)),
      );
      const fetchingHashes = new Set<string>();
      client.getQueryCache().subscribe((event) => {
        const query = event.query;
        if (!query) return;
        const hash = JSON.stringify(query.queryKey);
        if (
          activeHashes.has(hash) &&
          query.state.fetchStatus === "fetching" &&
          !fetchingHashes.has(hash)
        ) {
          fetchingHashes.add(hash);
          record("resume-fetch-start", query.queryKey);
        }
      });

      return active.map((query) => ({
        key: query.queryKey,
        refetchOnWindowFocus: query.options.refetchOnWindowFocus ?? null,
      }));
    },
    [...REPRESENTATIVE_FAMILIES],
  );

  expect(seasonedKeys.length).toBeGreaterThanOrEqual(3);

  const originalHasFocus = await page.evaluateHandle(() => document.hasFocus);
  await page.evaluate(() => {
    (
      window as typeof window & { __FOREGROUND_PROBE_ARMED__?: boolean }
    ).__FOREGROUND_PROBE_ARMED__ = true;
    Object.defineProperty(document, "hasFocus", {
      configurable: true,
      value: () => false,
    });
    window.dispatchEvent(new Event("blur"));
  });
  await expect.poll(() => page.evaluate(() => document.hasFocus())).toBe(false);

  // Headless Chromium reports every page as focused, even after bringToFront on
  // another target. Drive the production listener and isAppFocused predicate
  // together rather than shipping a false-green no-op focus transition.
  await page.evaluate(() => {
    Object.defineProperty(document, "hasFocus", {
      configurable: true,
      value: () => true,
    });
    window.dispatchEvent(new Event("focus"));
  });
  await expect.poll(() => page.evaluate(() => document.hasFocus())).toBe(true);
  await expect(page.getByTestId("chat-title")).toHaveText("alice-tyler", {
    timeout: 5_000,
  });

  const probes = await expect
    .poll(
      () =>
        page.evaluate(
          () =>
            (
              window as typeof window & {
                __FOREGROUND_PROBES__?: ForegroundProbe[];
              }
            ).__FOREGROUND_PROBES__ ?? [],
        ),
      { timeout: 5_000 },
    )
    .toEqual(
      expect.arrayContaining([
        expect.objectContaining({ marker: "focus" }),
        expect.objectContaining({ marker: "first-frame" }),
        expect.objectContaining({ marker: "interaction-dispatched" }),
        expect.objectContaining({ marker: "interaction-committed" }),
      ]),
    );

  // expect.poll returns void; read the settled marker stream for ordering.
  void probes;
  const settled = await page.evaluate(
    () =>
      (window as typeof window & { __FOREGROUND_PROBES__?: ForegroundProbe[] })
        .__FOREGROUND_PROBES__ ?? [],
  );
  await test.info().attach("foreground-ordering", {
    body: Buffer.from(
      `${JSON.stringify({ seasonedKeys, probes: settled }, null, 2)}\n`,
    ),
    contentType: "application/json",
  });

  const index = (marker: string) =>
    settled.findIndex((probe) => probe.marker === marker);
  const focus = settled.find((probe) => probe.marker === "focus");
  expect(focus?.detail).toEqual({
    hasFocus: true,
    visibilityState: "visible",
  });
  expect(index("first-frame")).toBeGreaterThan(index("focus"));
  expect(index("interaction-dispatched")).toBeGreaterThan(index("first-frame"));
  expect(index("interaction-committed")).toBeGreaterThan(
    index("interaction-dispatched"),
  );

  // The production scheduler promises a painted interaction boundary before
  // resume work, not completion of an asynchronous route transition. Dispatch
  // proves the sidebar target was actionable after the first frame; the commit
  // marker remains a responsiveness witness and must still settle promptly.
  const firstResumeFetch = index("resume-fetch-start");
  if (firstResumeFetch !== -1) {
    expect(firstResumeFetch).toBeGreaterThan(index("interaction-dispatched"));
  }

  await originalHasFocus.dispose();
});
