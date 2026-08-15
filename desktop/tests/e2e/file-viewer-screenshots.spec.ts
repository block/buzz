import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

// Screenshot coverage for the attachment file viewer. Each shot is scoped to
// a distinct state (markdown render, code render, two-tab strip) so the PNGs
// are never byte-identical — see AGENTS.md "Distinct states".

const MARKDOWN_SHA = "a".repeat(64);
const MARKDOWN_URL = `http://localhost:3000/media/${MARKDOWN_SHA}.md`;
const MARKDOWN_BODY = [
  "# Release notes",
  "",
  "Shipped the **file viewer** panel.",
  "",
  "## Highlights",
  "",
  "- Tabs for every opened attachment",
  "- Drag the divider to resize",
  "- Markdown and code rendering",
  "",
  "| Surface | Supported |",
  "| --- | --- |",
  "| Channels | yes |",
  "| DMs | yes |",
  "| Threads | yes |",
].join("\n");

const SCRIPT_SHA = "b".repeat(64);
const SCRIPT_URL = `http://localhost:3000/media/${SCRIPT_SHA}.sh`;
const SCRIPT_BODY = [
  "#!/usr/bin/env bash",
  "# Apply the non-secret configuration.",
  "set -euo pipefail",
  "",
  'C="docker compose exec -T hermes hermes"',
  "",
  'echo "==> Buzz: transport and access"',
  "$C config set gateway.platforms.buzz.enabled true",
  "$C config set gateway.platforms.buzz.extra.poll_interval 4",
].join("\n");

// A third file with a long name, so the strip overflows and its scrollbar and
// active-tab fill can be inspected — the real-world case is an agent
// delivering a bundle of files at once.
const DATA_SHA = "c".repeat(64);
const DATA_URL = `http://localhost:3000/media/${DATA_SHA}.json`;
const DATA_BODY = JSON.stringify(
  { generated: "2026-08-12", rows: [{ date: "2026-08-01", breadth: 0.62 }] },
  null,
  2,
);

function imetaTag(url: string, mime: string, sha: string, filename: string) {
  return [
    "imeta",
    `url ${url}`,
    `m ${mime}`,
    `x ${sha}`,
    `filename ${filename}`,
  ];
}

async function emitBundleMessage(page: Page) {
  await page.waitForFunction(
    () =>
      typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function" &&
      (window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
        channelName: "general",
      }) ??
        false),
  );
  await page.evaluate(
    ({ dataUrl, markdownUrl, scriptUrl, tags }) => {
      const emit = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
      if (!emit) throw new Error("Mock message emitter is unavailable.");
      emit({
        channelName: "general",
        content: [
          "Here is the release bundle",
          "",
          `[release-notes.md](${markdownUrl})`,
          "",
          `[apply-config.sh](${scriptUrl})`,
          "",
          `[market_breadth_history.json](${dataUrl})`,
        ].join("\n"),
        extraTags: tags,
      });
    },
    {
      dataUrl: DATA_URL,
      markdownUrl: MARKDOWN_URL,
      scriptUrl: SCRIPT_URL,
      tags: [
        imetaTag(
          MARKDOWN_URL,
          "text/markdown",
          MARKDOWN_SHA,
          "release-notes.md",
        ),
        imetaTag(SCRIPT_URL, "application/x-sh", SCRIPT_SHA, "apply-config.sh"),
        imetaTag(
          DATA_URL,
          "application/octet-stream",
          DATA_SHA,
          "market_breadth_history.json",
        ),
      ],
    },
  );
}

test("file viewer states", async ({ page }) => {
  await installMockBridge(page);
  await page.route(MARKDOWN_URL, (route) =>
    route.fulfill({ body: MARKDOWN_BODY, contentType: "text/markdown" }),
  );
  await page.route(SCRIPT_URL, (route) =>
    route.fulfill({ body: SCRIPT_BODY, contentType: "text/x-shellscript" }),
  );
  await page.route(DATA_URL, (route) =>
    route.fulfill({ body: DATA_BODY, contentType: "application/octet-stream" }),
  );

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await emitBundleMessage(page);

  // 1. File cards in the timeline, before anything is opened.
  const cards = page.getByTestId("file-card");
  await expect(cards).toHaveCount(3);
  await waitForAnimations(page);
  await page.screenshot({
    path: "test-results/file-viewer/01-file-cards.png",
    clip: { height: 340, width: 700, x: 380, y: 380 },
  });

  // 2. Markdown rendered in the viewer panel.
  await cards.first().click();
  const panel = page.getByTestId("file-viewer-panel");
  await expect(
    panel.getByRole("heading", { name: "Release notes" }),
  ).toBeVisible();
  await waitForAnimations(page);
  await panel.screenshot({
    path: "test-results/file-viewer/02-markdown.png",
  });

  // 3. Second file opened: two tabs, syntax-highlighted shell script.
  await cards.nth(1).click();
  await expect(page.getByTestId("file-viewer-tab")).toHaveCount(2);
  await expect(page.getByTestId("file-viewer-code")).toContainText(
    "set -euo pipefail",
  );
  await waitForAnimations(page);
  await panel.screenshot({
    path: "test-results/file-viewer/03-code-two-tabs.png",
  });

  // 4. Resized wide, back on the markdown tab.
  await page.getByTestId("file-viewer-tab").first().click();
  await expect(page.getByTestId("file-viewer-markdown")).toBeVisible();
  const handle = page.getByTestId("right-auxiliary-pane-resize-handle");
  const box = await handle.boundingBox();
  if (!box) throw new Error("Resize handle has no bounding box.");
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x - 240, box.y + box.height / 2, { steps: 12 });
  await page.mouse.up();
  await waitForAnimations(page);
  await page.screenshot({
    path: "test-results/file-viewer/04-resized-wide.png",
  });

  // 5. Three files open in a narrow panel: the strip overflows, so this is
  //    where the hairline scrollbar and the filled active chip are legible.
  await page.getByTestId("right-auxiliary-pane-resize-handle").dblclick();
  await cards.nth(2).click();
  await expect(page.getByTestId("file-viewer-tab")).toHaveCount(3);
  await expect(page.getByTestId("file-viewer-code")).toContainText("breadth");
  await waitForAnimations(page);
  await panel.screenshot({
    path: "test-results/file-viewer/05-three-tabs-overflow.png",
  });
});
