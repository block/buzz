import { expect, test, type Page } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const SHOTS = "test-results/release-run-preview";
const ARTWORKS = [
  "https://assets.trakd.test/total-unison.svg",
  "https://assets.trakd.test/assumptions.svg",
  "https://assets.trakd.test/love-trance.svg",
] as const;

const releaseRun = {
  version: 1,
  runId: "deezer-reidentify:2026-08-22T09:30:00.000Z",
  runName: "Daily release check",
  status: "completed",
  checked: 41,
  released: 3,
  held: 38,
  sourceHealth: "Deezer verified all 3 tracks",
  finishedAt: "2026-08-22T09:30:00.000Z",
  tracks: [
    {
      id: "d-stone-total-unison",
      artist: "D Stone",
      title: "Total Unison",
      version: "Original Mix",
      label: "Heist Recordings",
      releaseDate: "Aug 22, 2026",
      artworkUrl: ARTWORKS[0],
      source: "Deezer",
      sourceUrl: "https://www.deezer.com/track/1",
      detailsUrl: "https://trakd.app/releases/d-stone-total-unison",
    },
    {
      id: "jamback-assumptions",
      artist: "Jamback",
      title: "Assumptions",
      version: "Extended Mix",
      label: "PIV",
      releaseDate: "Aug 22, 2026",
      artworkUrl: ARTWORKS[1],
      source: "Deezer",
      sourceUrl: "https://www.deezer.com/track/2",
      detailsUrl: "https://trakd.app/releases/jamback-assumptions",
    },
    {
      id: "silva-bumpa-love-trance",
      artist: "Silva Bumpa",
      title: "Love Trance",
      label: "Hardline Sounds",
      releaseDate: "Aug 22, 2026",
      artworkUrl: ARTWORKS[2],
      source: "Deezer",
      sourceUrl: "https://www.deezer.com/track/3",
      detailsUrl: "https://trakd.app/releases/silva-bumpa-love-trance",
    },
  ],
} as const;

const releaseLink = `buzz://release-run?data=${Buffer.from(
  JSON.stringify(releaseRun),
).toString("base64url")}`;

async function openChannelWithReleaseReport(page: Page) {
  await page.route("https://assets.trakd.test/**", async (route) => {
    const index = ARTWORKS.indexOf(
      route.request().url() as (typeof ARTWORKS)[number],
    );
    const palettes = [
      ["#FE6A86", "#3D101A"],
      ["#F3BB52", "#522E0B"],
      ["#70D6C7", "#103B3A"],
    ];
    const [start, end] = palettes[Math.max(index, 0)];
    await route.fulfill({
      body: `<svg xmlns="http://www.w3.org/2000/svg" width="160" height="160" viewBox="0 0 160 160"><defs><linearGradient id="g" x2="1" y2="1"><stop stop-color="${start}"/><stop offset="1" stop-color="${end}"/></linearGradient></defs><rect width="160" height="160" fill="url(#g)"/><circle cx="80" cy="80" r="48" fill="none" stroke="white" stroke-opacity=".66" stroke-width="2"/><circle cx="80" cy="80" r="8" fill="white" fill-opacity=".82"/><path d="M18 122C48 93 94 139 145 72" fill="none" stroke="white" stroke-opacity=".5" stroke-width="5"/></svg>`,
      contentType: "image/svg+xml",
    });
  });
  await installMockBridge(page);
  await page.setViewportSize({ width: 1120, height: 820 });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("channel-general").click();
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
  );
  await page.evaluate(
    ({ content, pubkey }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        pubkey,
        content,
      });
    },
    {
      content: [
        "**Release run update**",
        "Checked 41 · Released 3 · Held 38",
        "",
        `[Open 3 releases](${releaseLink})`,
      ].join("\n"),
      pubkey: TEST_IDENTITIES.alice.pubkey,
    },
  );
  const row = page
    .getByTestId("message-row")
    .filter({ hasText: "Release run update" })
    .last();
  await expect(row).toBeVisible();
  return row;
}

test("release report opens its exact run in an anchored glass preview", async ({
  page,
}) => {
  const row = await openChannelWithReleaseReport(page);
  await row.locator("[data-release-run-trigger]").click();

  const preview = page.getByTestId("release-run-popover");
  await expect(preview).toBeVisible();
  await expect(preview).toContainText("Released · 3 tracks");
  await expect(preview.locator("[data-release-track]")).toHaveCount(3);
  await expect(preview).toContainText("Total Unison");
  await expect(preview).toContainText("Assumptions");
  await expect(preview).toContainText("Love Trance");
  await expect(preview).toContainText("Deezer verified all 3 tracks");

  await waitForAnimations(page);
  await page.screenshot({
    animations: "disabled",
    path: `${SHOTS}/desktop.png`,
  });
});

test("the same release report becomes a bottom sheet on a narrow window", async ({
  page,
}) => {
  const row = await openChannelWithReleaseReport(page);
  await page.setViewportSize({ width: 620, height: 820 });
  await row.locator("[data-release-run-trigger]").click();

  const preview = page.getByTestId("release-run-sheet");
  await expect(preview).toBeVisible();
  await expect(preview.locator("[data-release-track]")).toHaveCount(3);

  await waitForAnimations(page);
  await page.screenshot({
    animations: "disabled",
    path: `${SHOTS}/narrow.png`,
  });
});
