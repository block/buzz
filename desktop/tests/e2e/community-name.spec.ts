import { expect, test, type Page } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const RELAY_A = "ws://localhost:3000";
const RELAY_B = "ws://localhost:3001";
type Profiles = Record<string, { name: string | null } | null>;

async function setup(page: Page, profiles: Profiles, isOffline = () => false) {
  await page.route("**/__test-community-profile**", async (route) => {
    if (isOffline()) return route.fulfill({ status: 503, body: "offline" });
    const url = new URL(route.request().url());
    const relay = url.searchParams.get("relay") ?? RELAY_A;
    if (route.request().method() === "POST") {
      const { name } = route.request().postDataJSON();
      profiles[relay] = { name };
    }
    await route.fulfill({ json: profiles[relay] ?? null });
  });
  await page.addInitScript(
    ({ a, b }) => {
      if (!localStorage.getItem("buzz-communities")) {
        localStorage.setItem(
          "buzz-communities",
          JSON.stringify([
            {
              id: "a",
              name: "Local Dev",
              relayUrl: a,
              addedAt: "2026-01-01",
            },
            {
              id: "b",
              name: "Local Dev",
              relayUrl: b,
              addedAt: "2026-01-02",
            },
          ]),
        );
        localStorage.setItem("buzz-active-community-id", "a");
      }
      type Invoke = (
        command: string,
        args: { relayUrl?: string; message?: { type: string; data: string } },
        options: unknown,
      ) => Promise<unknown>;
      let invoke: Invoke;
      const internals = {};
      Object.defineProperty(internals, "invoke", {
        configurable: true,
        get:
          () =>
          async (
            command: string,
            args: {
              relayUrl?: string;
              message?: { type: string; data: string };
            },
            options: unknown,
          ) => {
            if (command === "fetch_workspace_profile") {
              const response = await fetch(
                `/__test-community-profile?relay=${encodeURIComponent(args.relayUrl ?? "")}`,
              );
              if (!response.ok) throw new Error("Relay unavailable");
              return response.json();
            }
            if (command === "fetch_workspace_icon") return null;
            if (
              command === "plugin:websocket|send" &&
              args.message?.type === "Text"
            ) {
              const [type, event] = JSON.parse(args.message.data);
              if (type === "EVENT" && event.kind === 9033) {
                const name = event.tags.find(
                  (tag: string[]) => tag[0] === "name",
                )?.[1];
                if (name)
                  await fetch(
                    `/__test-community-profile?relay=${encodeURIComponent(a)}`,
                    {
                      method: "POST",
                      body: JSON.stringify({ name }),
                      headers: { "Content-Type": "application/json" },
                    },
                  );
              }
            }
            return invoke(command, args, options);
          },
        set: (next: Invoke) => {
          invoke = next;
        },
      });
      (
        window as unknown as { __TAURI_INTERNALS__: unknown }
      ).__TAURI_INTERNALS__ = internals;
    },
    { a: RELAY_A, b: RELAY_B },
  );
  await installMockBridge(
    page,
    { relayRequiresMembership: true },
    {
      skipCommunitySeed: true,
    },
  );
  await page.goto("/");
}

async function openSettings(page: Page) {
  await page.getByTestId("community-rail-button-a").click({ button: "right" });
  await page.getByRole("menuitem", { name: "Community settings" }).click();
  return page.getByRole("dialog", { name: "Edit Community" });
}

test("two clean installations show a shared name and reconcile a rename after focus", async ({
  page,
  browser,
}) => {
  const profiles = {
    [RELAY_A]: { name: "ATL BitLab" },
    [RELAY_B]: { name: "Other community" },
  };
  const second = await browser.newPage();
  try {
    await setup(page, profiles);
    await setup(second, profiles);
    for (const client of [page, second])
      await expect(client.getByTestId("sidebar-profile-card")).toContainText(
        "ATL BitLab",
      );
    const dialog = await openSettings(page);
    await dialog
      .getByLabel("Community name", { exact: true })
      .fill("BitLab Builders");
    await dialog
      .getByRole("button", { name: "Save name", exact: true })
      .click();
    await expect(dialog.getByRole("status")).toHaveText(
      "Community name saved.",
    );
    await second.evaluate(() => window.dispatchEvent(new Event("focus")));
    await expect(second.getByTestId("sidebar-profile-card")).toContainText(
      "BitLab Builders",
    );
    await expect(
      dialog.getByText(/Previous device label: Local Dev/),
    ).toBeVisible();
    await waitForAnimations(page);
    await dialog.screenshot({
      path: "test-results/community-names/01-shared-name-settings.png",
    });
    await dialog.getByLabel("Local nickname (optional)").fill("My nickname");
    await dialog.getByRole("button", { name: "Save Changes" }).click();
    await expect(page.getByTestId("sidebar-profile-card")).toContainText(
      "My nickname",
    );
    await expect(second.getByTestId("sidebar-profile-card")).toContainText(
      "BitLab Builders",
    );
    await page.getByTestId("community-rail-button-b").click();
    await expect(page.getByTestId("sidebar-profile-card")).toContainText(
      "Other community",
    );
    await waitForAnimations(second);
    await second.getByTestId("sidebar-profile-card").screenshot({
      path: "test-results/community-names/02-second-installation.png",
    });
  } finally {
    await second.close();
  }
});

test("offline restart retains cached name and legacy relays cannot receive a name command", async ({
  page,
}) => {
  const profiles: Profiles = {
    [RELAY_A]: { name: "Shared cached name" },
    [RELAY_B]: null,
  };
  let offline = false;
  await setup(page, profiles, () => offline);
  await expect(page.getByTestId("sidebar-profile-card")).toContainText(
    "Shared cached name",
  );
  offline = true;
  await page.reload();
  await expect(page.getByTestId("sidebar-profile-card")).toContainText(
    "Shared cached name",
  );
  offline = false;
  profiles[RELAY_A] = null;
  const dialog = await openSettings(page);
  await expect(
    dialog.getByText(
      "Upgrade this relay to share a community name across devices.",
    ),
  ).toBeVisible();
  await expect(
    dialog.getByRole("button", { name: "Save name", exact: true }),
  ).toHaveCount(0);
  await expect(page.getByTestId("sidebar-profile-card")).toContainText(
    "Shared cached name",
  );
});
