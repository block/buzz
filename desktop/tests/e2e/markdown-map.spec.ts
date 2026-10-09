import { expect, type Page, test } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";

const spec = {
  height: 320,
  markers: [
    { lng: -122.4, lat: 37.79, label: "Store A", color: "#e11d48" },
    { lng: -122.45, lat: 37.76, label: "Store B" },
  ],
  lines: [
    {
      coordinates: [
        [-122.4, 37.79],
        [-122.45, 37.76],
      ],
    },
  ],
};

// Keep the test hermetic: serve a blank style instead of remote tiles.
async function stubTiles(page: Page) {
  await page.route("https://tiles.openfreemap.org/**", (route) =>
    route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        version: 8,
        sources: {},
        layers: [
          {
            id: "bg",
            type: "background",
            paint: { "background-color": "#dde" },
          },
        ],
      }),
    }),
  );
}

async function emitMessage(page: Page, content: string) {
  return page.evaluate((body) => {
    const root = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "general",
      content: body,
    });
    if (!root) throw new Error("Mock message was not emitted");
    return root.id;
  }, content);
}

test("```map blocks render an interactive map and report bad specs", async ({
  page,
}) => {
  await stubTiles(page);
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await page.waitForFunction(() =>
    window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
      channelName: "general",
    }),
  );

  const mapId = await emitMessage(
    page,
    `Route plan\n\n\`\`\`map\n${JSON.stringify(spec, null, 2)}\n\`\`\``,
  );
  const message = page.locator(`[data-message-id="${mapId}"]`);
  const map = message.getByTestId("markdown-map-block");
  await expect(map).toBeVisible();
  await expect(map).toHaveCSS("height", "320px");
  await expect(map.locator("canvas.maplibregl-canvas")).toBeVisible();
  await expect(map.locator(".maplibregl-marker")).toHaveCount(2);
  await expect(map.locator('[title="Store A"]')).toBeVisible();
  await expect(message.locator("[data-code-block]")).toHaveCount(0);

  const badId = await emitMessage(
    page,
    '```map\n{"markers": [{"lng": 500}]}\n```',
  );
  await expect(
    page
      .locator(`[data-message-id="${badId}"]`)
      .getByText("markers[0] needs valid lng/lat."),
  ).toBeVisible();
});
