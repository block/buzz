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
    engagement: 20,
    lane: "important",
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
    engagement: 10,
    lane: "important",
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

/** A live discussion nobody is asking the viewer about. */
const HOT_FIBRE = {
  ...FIBRES[1],
  id: "f3",
  kind: "fyi",
  score: 34,
  engagement: 78,
  lane: "hot",
  title: "Design is going back and forth on the collapsed nav",
  summary: "Five people are weighing tap targets against discoverability.",
  why: "A busy thread on a surface you have worked on before.",
  whyShort: "Busy design thread.",
};

function payload(open: typeof FIBRES, done: typeof FIBRES = []) {
  return {
    fibres: open,
    done,
    openCount: open.length,
    doneCount: done.length,
    clearedCount: done.length,
    laneCounts: {
      important: open.filter((fibre) => fibre.lane === "important").length,
      hot: open.filter((fibre) => fibre.lane === "hot").length,
      other: open.filter((fibre) => fibre.lane === "other").length,
    },
    ingested: 0,
    changes: [],
  };
}

async function mockFibreService(
  page: import("@playwright/test").Page,
  initial = [...FIBRES],
) {
  let open = [...initial];
  let done: typeof FIBRES = [];
  const notes: string[] = [];
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
        await route.fulfill({ headers: CORS, json: payload(open, done) });
        return;
      }
      if (url.pathname === "/ingest" && method === "POST") {
        await route.fulfill({ headers: CORS, json: payload(open, done) });
        return;
      }
      if (url.pathname === "/fibres/restore" && method === "POST") {
        open = [...FIBRES];
        done = [];
        await route.fulfill({ headers: CORS, json: payload(open, done) });
        return;
      }
      if (url.pathname === "/feedback" && method === "POST") {
        await route.fulfill({
          headers: CORS,
          json: { feedback: { id: "fb-1" } },
        });
        return;
      }
      if (method === "PATCH" && url.pathname.startsWith("/feedback/")) {
        const body = JSON.parse(route.request().postData() ?? "{}") as {
          note?: string;
        };
        notes.push(body.note ?? "");
        await route.fulfill({
          headers: CORS,
          json: { feedback: { id: "fb-1" } },
        });
        return;
      }
      if (method === "PATCH" && url.pathname.startsWith("/fibres/")) {
        const id = url.pathname.split("/").at(-1);
        const body = JSON.parse(route.request().postData() ?? "{}") as {
          status?: string;
        };
        const current =
          open.find((fibre) => fibre.id === id) ??
          done.find((fibre) => fibre.id === id);
        open = open.filter((fibre) => fibre.id !== id);
        done = done.filter((fibre) => fibre.id !== id);
        if (current && body.status === "done") {
          done = [{ ...current, status: "done" }, ...done];
        } else if (current && body.status === "open") {
          open = [{ ...current, status: "open" }, ...open];
        }
        await route.fulfill({
          headers: CORS,
          json: {
            fibre: current ? { ...current, status: body.status } : null,
            ...payload(open, done),
          },
        });
        return;
      }
      await route.fulfill({ headers: CORS, json: {} });
    },
  );
  return { notes };
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
  await expect(page.getByTestId("fibre-detail")).not.toContainText(
    "Why this ranks here",
  );
  await expect(page.getByTestId("fibre-artifacts")).toContainText(
    "Source artifacts",
  );

  await page.getByTestId("fibre-why-trigger").hover();
  await expect(page.getByTestId("fibre-why-card")).toBeVisible();
  await expect(page.getByTestId("fibre-why-card")).toContainText(
    "Why this ranks here",
  );

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

test("fibre inbox empty open state has no Inbox Zero copy", async ({
  page,
}) => {
  await installMockBridge(page);
  await mockFibreService(page, []);
  await page.goto("/");
  await expect(page.getByTestId("fibre-inbox")).toBeVisible();
  await expect(page.getByTestId("fibre-zero")).toBeVisible();
  await expect(page.getByTestId("fibre-zero")).toHaveText("");
  await expect(page.getByTestId("fibre-restore")).toHaveCount(0);
  await expect(page.getByTestId("fibre-tab-important-count")).toHaveText("0");
  await expect(page.getByTestId("fibre-lane-empty")).toHaveCount(0);
  await expect(page.locator("html")).toHaveAttribute("data-inbox-zero", "");
});

test("lanes route fibres and each tab counts its own", async ({ page }) => {
  await installMockBridge(page);
  await mockFibreService(page, [...FIBRES, HOT_FIBRE]);
  await page.goto("/");
  await expect(page.getByTestId("fibre-inbox")).toBeVisible();

  await expect(page.getByTestId("fibre-tab-important-count")).toHaveText("2");
  await expect(page.getByTestId("fibre-tab-hot-count")).toHaveText("1");
  await expect(page.getByTestId("fibre-tab-other-count")).toHaveText("0");
  await expect(page.getByTestId("fibre-row")).toHaveCount(2);

  await page.getByTestId("fibre-tab-hot").click();
  await expect(page.getByTestId("fibre-row")).toHaveCount(1);
  await expect(page.getByTestId("fibre-detail")).toContainText("collapsed nav");

  await page.getByTestId("fibre-tab-other").click();
  await expect(page.getByTestId("fibre-row")).toHaveCount(0);
  await expect(page.getByTestId("fibre-lane-empty")).toContainText(
    "Nothing else waiting",
  );
  // Other lanes still hold fibres, so this is not inbox zero.
  await expect(page.locator("html")).not.toHaveAttribute("data-inbox-zero");
});

test("dismissing offers to record why, and the reason reaches the engine", async ({
  page,
}) => {
  await installMockBridge(page);
  const { notes } = await mockFibreService(page);
  await page.goto("/");
  await expect(page.getByTestId("fibre-row")).toHaveCount(2);

  await page.getByTestId("fibre-dismiss").click();
  await expect(page.getByTestId("fibre-row")).toHaveCount(1);

  await page.getByRole("button", { name: "Add reason" }).click();
  await expect(page.getByTestId("fibre-reason-dialog")).toBeVisible();
  await page
    .getByTestId("fibre-reason-input")
    .fill("Incident channel noise, the on-call owns this");
  await page.getByTestId("fibre-reason-save").click();

  await expect(page.getByTestId("fibre-reason-dialog")).toHaveCount(0);
  await expect
    .poll(() => notes)
    .toEqual(["Incident channel noise, the on-call owns this"]);
});

test("fibre inbox Done tab lists completed fibres", async ({ page }) => {
  await installMockBridge(page);
  await mockFibreService(page);
  await page.goto("/");
  await expect(page.getByTestId("fibre-row")).toHaveCount(2);
  await page.getByTestId("fibre-done").click();
  await expect(page.getByTestId("fibre-row")).toHaveCount(1);
  await page.getByTestId("fibre-tab-done").click();
  await expect(page.getByTestId("fibre-row")).toHaveCount(1);
  await expect(page.getByTestId("fibre-reopen")).toBeVisible();
  await expect(page.locator("html")).not.toHaveAttribute("data-inbox-zero");
});
