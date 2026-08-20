/**
 * Fibre Inbox smoke coverage. The triage service is mocked so CI does not
 * need OpenAI or a running fibre engine.
 *
 * Run: pnpm build:e2e && pnpm exec playwright test --project=smoke \
 *        tests/e2e/fibre-inbox.spec.ts
 */
import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/fibre-inbox";

const CORS = {
  "access-control-allow-origin": "*",
  "access-control-allow-methods": "GET, POST, PATCH, OPTIONS",
  "access-control-allow-headers": "content-type",
  "content-type": "application/json",
};

const FIBRES = [
  {
    id: "f1",
    kind: "blocker",
    status: "open",
    score: 98,
    title:
      "Incident root cause is identified — the rollback call is waiting on you",
    summary: "The agent traced the degradation and posted findings.",
    why: "An agent @-mentioned you with a finding marked ROOT CAUSE.",
    whyShort: "Unanswered agent @mention on the code path you own.",
    signals: [{ weight: "+34", label: "Direct @mention, unanswered" }],
    channelId: "war-room",
    channelName: "war-room",
    isDm: false,
    people: [{ pubkey: "aa", label: "Incident Responder" }],
    artifacts: [
      {
        eventId: "evt-1",
        channelId: "war-room",
        channelName: "war-room",
        threadRootId: "evt-1",
        authorPubkey: "aa",
        authorLabel: "Incident Responder",
        content: "@jacob FINDINGS — ROOT CAUSE IDENTIFIED!!",
        createdAt: Math.floor(Date.now() / 1000) - 41 * 60,
        isDm: false,
      },
    ],
    createdAt: Math.floor(Date.now() / 1000) - 41 * 60,
    updatedAt: Math.floor(Date.now() / 1000) - 41 * 60,
  },
  {
    id: "f2",
    kind: "ask",
    status: "open",
    score: 84,
    title: "Vlad needs you to run the triage scripts before the next build",
    summary: "Two scripts have to run in order.",
    why: "A direct @mention containing an executable instruction.",
    whyShort: "Unanswered instruction that blocks two teammates.",
    signals: [{ weight: "+29", label: "Direct @mention, unanswered" }],
    channelId: "hack",
    channelName: "hack-project-mesh",
    isDm: false,
    people: [{ pubkey: "bb", label: "Vlad" }],
    artifacts: [
      {
        eventId: "evt-2",
        channelId: "hack",
        channelName: "hack-project-mesh",
        threadRootId: "evt-2",
        authorPubkey: "bb",
        authorLabel: "Vlad",
        content: "@jacob fyi, the above scripts are to run the triage",
        createdAt: Math.floor(Date.now() / 1000) - 3600,
        isDm: false,
      },
    ],
    createdAt: Math.floor(Date.now() / 1000) - 3600,
    updatedAt: Math.floor(Date.now() / 1000) - 3600,
  },
];

function payload(open: typeof FIBRES) {
  return {
    fibres: open,
    openCount: open.length,
    clearedCount: FIBRES.length - open.length,
    ingested: 0,
    changes: [],
  };
}

async function mockFibreService(
  page: import("@playwright/test").Page,
  initial = [...FIBRES],
) {
  let open = [...initial];
  await page.route(
    /http:\/\/(?:localhost|127\.0\.0\.1):8787\/.*/,
    async (route) => {
      if (route.request().method() === "OPTIONS") {
        await route.fulfill({ status: 204, headers: CORS });
        return;
      }
      const url = new URL(route.request().url());
      const method = route.request().method();
      if (url.pathname === "/health") {
        await route.fulfill({ headers: CORS, json: { status: "ok" } });
        return;
      }
      if (url.pathname === "/fibres" && method === "GET") {
        await route.fulfill({ headers: CORS, json: payload(open) });
        return;
      }
      if (url.pathname === "/ingest" && method === "POST") {
        await route.fulfill({ headers: CORS, json: payload(open) });
        return;
      }
      if (url.pathname === "/fibres/restore" && method === "POST") {
        open = [...FIBRES];
        await route.fulfill({ headers: CORS, json: payload(open) });
        return;
      }
      if (method === "PATCH" && url.pathname.startsWith("/fibres/")) {
        const id = url.pathname.split("/").at(-1);
        open = open.filter((fibre) => fibre.id !== id);
        await route.fulfill({
          headers: CORS,
          json: {
            fibre: FIBRES.find((fibre) => fibre.id === id) ?? null,
            ...payload(open),
          },
        });
        return;
      }
      await route.fulfill({ headers: CORS, json: {} });
    },
  );
}

test("fibre inbox lists scored fibres and opens detail", async ({ page }) => {
  await installMockBridge(page);
  await mockFibreService(page);
  await page.setViewportSize({ width: 1280, height: 720 });
  await page.goto("/");
  await expect(page.getByTestId("fibre-inbox")).toBeVisible();
  await expect(page.getByTestId("fibre-row")).toHaveCount(2);
  await expect(page.getByTestId("fibre-detail")).toContainText(
    "Incident root cause",
  );
  await expect(page.getByTestId("sidebar-home-count")).toHaveText("2");
  await expect(page.getByTestId("fibre-row").first()).toHaveAttribute(
    "data-kind",
    "blocker",
  );

  await page.getByTestId("fibre-row").nth(1).click();
  await expect(page.getByTestId("fibre-detail")).toContainText(
    "Vlad needs you",
  );
  await expect(page.getByTestId("fibre-detail")).toContainText("Ask");

  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOTS}/01-fibre-inbox.png` });
});

test("fibre inbox Done removes the selected fibre", async ({ page }) => {
  await installMockBridge(page);
  await mockFibreService(page);
  await page.goto("/");
  await expect(page.getByTestId("fibre-row")).toHaveCount(2);
  await page.getByTestId("fibre-done").click();
  await expect(page.getByTestId("fibre-row")).toHaveCount(1);
});

test("fibre inbox keyboard Done marks the selected fibre", async ({ page }) => {
  await installMockBridge(page);
  await mockFibreService(page);
  await page.goto("/");
  await expect(page.getByTestId("fibre-row")).toHaveCount(2);
  await page.locator("body").click({ position: { x: 400, y: 200 } });
  await page.keyboard.press("e");
  await expect(page.getByTestId("fibre-row")).toHaveCount(1);
});

test("fibre inbox empty state is Fibre Zero", async ({ page }) => {
  await installMockBridge(page);
  await mockFibreService(page, []);
  await page.goto("/");
  await expect(page.getByTestId("fibre-inbox")).toBeVisible();
  await expect(page.getByTestId("fibre-zero")).toBeVisible();
  await expect(page.getByTestId("fibre-zero")).toContainText("Fibre Zero");
  await expect(page.getByTestId("fibre-restore")).toBeVisible();
});
