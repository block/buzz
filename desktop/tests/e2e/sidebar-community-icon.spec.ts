import { expect, test, type Page } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const RELAY_A = "ws://localhost:3000";
const RELAY_B = "ws://localhost:3001";
const ICON = `data:image/svg+xml,${encodeURIComponent(
  '<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128"><rect width="128" height="128" rx="28" fill="#F6534F"/><text x="64" y="88" text-anchor="middle" font-size="80">⚡</text></svg>',
)}`;

async function setup(page: Page, icon: string | null, cachedIcon?: string) {
  await page.addInitScript(
    ({ relayA, relayB, icon, cachedIcon }) => {
      localStorage.setItem(
        "buzz-communities",
        JSON.stringify([
          {
            id: "a",
            name: "ATL BitLab",
            relayUrl: relayA,
            addedAt: "2026-01-01",
          },
          {
            id: "b",
            name: "Other community",
            relayUrl: relayB,
            addedAt: "2026-01-02",
          },
        ]),
      );
      localStorage.setItem("buzz-active-community-id", "a");
      if (cachedIcon) {
        localStorage.setItem(
          "buzz-community-icons",
          JSON.stringify({ [relayA]: cachedIcon }),
        );
      }

      // Supply the native NIP-11 result before the first React render. All
      // other IPC continues through the standard E2E bridge.
      type Invoke = (
        command: string,
        args: Record<string, unknown>,
        options: unknown,
      ) => unknown;
      const internals: Record<string, unknown> = {};
      let invoke: Invoke;
      Object.defineProperty(internals, "invoke", {
        configurable: true,
        get:
          () =>
          (command: string, args: Record<string, unknown>, options: unknown) =>
            command === "fetch_workspace_icon"
              ? args.relayUrl !== relayA
                ? Promise.resolve(null)
                : cachedIcon
                  ? (
                      window as unknown as {
                        __testFetchCommunityIcon: () => Promise<string | null>;
                      }
                    ).__testFetchCommunityIcon()
                  : Promise.resolve(icon)
              : invoke(command, args, options),
        set: (next: Invoke) => {
          invoke = next;
        },
      });
      (
        window as unknown as { __TAURI_INTERNALS__: unknown }
      ).__TAURI_INTERNALS__ = internals;
    },
    { relayA: RELAY_A, relayB: RELAY_B, icon, cachedIcon },
  );
  await installMockBridge(page, undefined, { skipCommunitySeed: true });
  await page.goto("/");
}

test("profile card and menu show the same relay icon, including on status hover", async ({
  page,
}) => {
  await setup(page, ICON);
  const card = page.getByTestId("sidebar-profile-card");
  const icon = card.locator('img[src^="data:image/svg+xml,"]');
  await expect(card).toContainText("ATL BitLab");
  await expect(icon).toHaveAttribute("src", ICON);
  await expect(icon).toHaveJSProperty("complete", true);
  expect(
    await icon.evaluate((element: HTMLImageElement) => element.naturalWidth),
  ).toBeGreaterThan(0);
  const box = await icon.boundingBox();
  expect(box?.width).toBe(box?.height);
  await waitForAnimations(page);
  await card.screenshot({
    path: "test-results/community-branding/01-profile-card.png",
  });

  await page.getByTestId("sidebar-profile-avatar-button").click();
  await expect(
    page.getByTestId("community-switcher").locator("img"),
  ).toHaveAttribute("src", ICON);
  await waitForAnimations(page);
  await page.getByTestId("profile-popover").screenshot({
    path: "test-results/community-branding/02-profile-menu.png",
  });
  await page.keyboard.press("Escape");

  await expect
    .poll(() =>
      page.evaluate(() =>
        window.__BUZZ_E2E_HAS_MOCK_GLOBAL_KIND_SUBSCRIPTION__?.(30315),
      ),
    )
    .toBe(true);
  await page.evaluate(() =>
    window.__BUZZ_E2E_SET_MOCK_USER_STATUS__?.({
      text: "Building",
      emoji: "🛠️",
    }),
  );
  await expect(page.getByTestId("sidebar-profile-user-status")).toContainText(
    "Building",
  );
  await card.hover();
  await expect(icon.locator("../../..")).toHaveCSS("opacity", "1");
  await expect(icon).toHaveAttribute("src", ICON);
  await waitForAnimations(page);
  await card.screenshot({
    path: "test-results/community-branding/03-status-hover.png",
  });

  await page.getByTestId("community-rail-button-b").click();
  await expect(card).toContainText("Other community");
  await expect(card.locator(`img[src="${ICON}"]`)).toHaveCount(0);
  await expect(card).toContainText("🐝");
});

test("profile card keeps the default icon when the relay has no icon", async ({
  page,
}) => {
  await setup(page, null);
  const card = page.getByTestId("sidebar-profile-card");
  await expect(card).toContainText("ATL BitLab");
  await expect(card).toContainText("🐝");
  await page.getByTestId("sidebar-profile-avatar-button").click();
  await expect(page.getByTestId("community-switcher")).toContainText("🐝");
});

for (const result of ["updated", "cleared"] as const) {
  test(`cached icon renders while fetching, then is ${result}`, async ({
    page,
  }) => {
    const freshIcon =
      result === "updated" ? ICON.replace("F6534F", "3399FF") : null;
    let resolveIcon!: (icon: string | null) => void;
    const response = new Promise<string | null>((resolve) => {
      resolveIcon = resolve;
    });
    await page.exposeFunction("__testFetchCommunityIcon", () => response);
    await setup(page, freshIcon, ICON);

    const card = page.getByTestId("sidebar-profile-card");
    const rail = page.getByTestId("community-rail-button-a");
    await expect(rail.locator("img")).toHaveAttribute("src", ICON);
    await expect(
      card.locator('img[src^="data:image/svg+xml,"]'),
    ).toHaveAttribute("src", ICON);
    await page.getByTestId("sidebar-profile-avatar-button").click();
    const menu = page.getByTestId("community-switcher");
    await expect(menu.locator("img")).toHaveAttribute("src", ICON);

    resolveIcon(freshIcon);
    for (const surface of [card, rail, menu]) {
      if (freshIcon) {
        await expect(
          surface.locator('img[src^="data:image/svg+xml,"]'),
        ).toHaveAttribute("src", freshIcon);
      } else {
        await expect(surface.locator(`img[src="${ICON}"]`)).toHaveCount(0);
      }
    }
    if (!freshIcon) {
      await expect(card).toContainText("🐝");
      await expect(menu).toContainText("🐝");
    }
    await expect
      .poll(() =>
        page.evaluate(
          (relay) =>
            JSON.parse(localStorage.getItem("buzz-community-icons") ?? "{}")[
              relay
            ] ?? null,
          RELAY_A,
        ),
      )
      .toBe(freshIcon);
  });
}
