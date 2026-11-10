import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const PROJECT_HOME_CHANNEL_ID = "cf63feec-21bb-5bf0-a2f8-0e4c3de8ec73";

async function enableProjectsFeature(page: import("@playwright/test").Page) {
  await page.addInitScript(() => {
    window.localStorage.setItem(
      "buzz-feature-overrides-v1",
      JSON.stringify({ projects: true }),
    );
  });
}

async function waitForProjectSnapshot(
  page: import("@playwright/test").Page,
): Promise<void> {
  await expect
    .poll(() =>
      page.evaluate(() =>
        Object.keys(window.localStorage).some((key) =>
          key.startsWith("buzz-projects.v1:"),
        ),
      ),
    )
    .toBe(true);
}

async function mutateProjectCache(
  page: import("@playwright/test").Page,
): Promise<void> {
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          "__BUZZ_E2E_QUERY_CLIENT__" in window &&
          Boolean(window.__BUZZ_E2E_QUERY_CLIENT__),
      ),
    )
    .toBe(true);
  await page.evaluate(() => {
    const queryClient = (
      window as typeof window & {
        __BUZZ_E2E_QUERY_CLIENT__?: {
          setQueryData: (
            key: readonly string[],
            updater: (current: unknown) => unknown,
          ) => void;
        };
      }
    ).__BUZZ_E2E_QUERY_CLIENT__;
    if (!queryClient) throw new Error("E2E query client is unavailable.");
    queryClient.setQueryData(["projects"], (current) =>
      Array.isArray(current) ? [...current] : current,
    );
  });
}

async function waitForProjectEnumeration(
  page: import("@playwright/test").Page,
): Promise<void> {
  await expect
    .poll(() =>
      page.evaluate(() => {
        const queryClient = window.__BUZZ_E2E_QUERY_CLIENT__;
        const state = queryClient?.getQueryState(["projects"]);
        return Boolean(
          state && state.fetchStatus === "idle" && state.dataUpdatedAt > 0,
        );
      }),
    )
    .toBe(true);
}

test("snapshot project home cannot publish repository healing", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await waitForProjectSnapshot(page);
  await page.getByTestId("channel-buzz").click();
  await expect(page.getByTestId("project-home-context-panel")).toBeVisible();

  await page.addInitScript(() => {
    window.__BUZZ_E2E_DEFER_FULL_PROJECT_QUERIES__ = true;
    window.__BUZZ_E2E_ACCEPTED_PROJECT_EVENTS__ = [];
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await mutateProjectCache(page);
  await page.getByTestId("channel-buzz").click();

  await expect(page.getByTestId("project-home-context-panel")).toBeVisible();
  await page.waitForTimeout(500);
  const projectPublications = await page.evaluate(
    () =>
      window.__BUZZ_E2E_ACCEPTED_PROJECT_EVENTS__?.filter(
        (event) =>
          event.kind === 30621 &&
          event.tags.some((tag) => tag[0] === "d" && tag[1] === "buzz"),
      ) ?? [],
  );
  expect(projectPublications).toEqual([]);
});

test("stale non-matching snapshot uses the scoped project-home lookup", async ({
  page,
}) => {
  await enableProjectsFeature(page);
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await waitForProjectSnapshot(page);

  await page.evaluate(() => {
    const key = Object.keys(window.localStorage).find((candidate) =>
      candidate.startsWith("buzz-projects.v1:"),
    );
    if (!key) throw new Error("Project snapshot was not persisted.");
    const snapshot = JSON.parse(window.localStorage.getItem(key) ?? "{}");
    snapshot.projects = snapshot.projects.filter(
      (project) => project.dtag !== "buzz",
    );
    const value = JSON.stringify([
      snapshot.ownerPubkey.toLowerCase(),
      snapshot.projects,
    ]);
    let integrity = 0x811c9dc5;
    for (let index = 0; index < value.length; index += 1) {
      integrity ^= value.charCodeAt(index);
      integrity = Math.imul(integrity, 0x01000193);
    }
    snapshot.integrity = (integrity >>> 0).toString(16).padStart(8, "0");
    window.localStorage.setItem(key, JSON.stringify(snapshot));
  });
  await page.addInitScript(() => {
    window.__BUZZ_E2E_DEFER_FULL_PROJECT_QUERIES__ = true;
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await mutateProjectCache(page);
  await page.getByTestId("channel-buzz").click();

  await expect(page.getByTestId("project-home-context-panel")).toBeVisible();
  const usedScopedLookup = await page.evaluate((channelId) => {
    return window.__BUZZ_E2E_PROJECT_QUERY_FILTERS__?.some((filter) =>
      filter["#buzz-channel"]?.includes(channelId),
    );
  }, PROJECT_HOME_CHANNEL_ID);
  expect(usedScopedLookup).toBe(true);
});

test("equal live project data enables healing after snapshot reconciliation", async ({
  page,
}) => {
  await page.addInitScript(() => {
    Date.now = () => 1_787_872_972_113;
  });
  await enableProjectsFeature(page);
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await waitForProjectSnapshot(page);

  await page.goto("/", { waitUntil: "domcontentloaded" });
  await waitForProjectEnumeration(page);
  await page.getByTestId("channel-buzz").click();

  await expect(page.getByTestId("project-channel-home")).toHaveAttribute(
    "data-repository-healing-enabled",
    "true",
  );
});
