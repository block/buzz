import { expect, test } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";
type PolicyWindow = Window & {
  __BUZZ_E2E_COMMAND_PAYLOADS__?: Array<{
    command: string;
    payload: { url?: string };
  }>;
  __BUZZ_E2E__: {
    mock: {
      openerError?: string;
      relayEntryPolicy: {
        required: boolean;
        requiredRelay?: string;
        error?: string;
      };
    };
  };
};

test("saved desktop community: required, retryable failure, recovery and keys survive", async ({
  page,
}) => {
  await installMockBridge(page, { relayEntryPolicy: { required: true } });
  await page.goto("/");
  const card = page.getByTestId("relay-entry-guidance");
  await expect(
    card.getByText("Upgrade Buzz to join this community"),
  ).toBeVisible();
  await expect(
    card.getByRole("button", { name: "Download latest Buzz" }),
  ).toBeVisible();
  await card.screenshot({ path: "test-results/relay-entry-required.png" });
  let storage = await page.evaluate(() => ({ ...localStorage }));
  // Native networking is the external boundary. The session and rendered gate stay real.
  await page.evaluate(() => {
    const config = (window as PolicyWindow).__BUZZ_E2E__;
    config.mock.relayEntryPolicy = { required: false, error: "HTTP 503" };
  });
  await card.getByRole("button", { name: "Retry community check" }).click();
  await expect(card).toBeHidden();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page.waitForFunction(
    () =>
      (
        window as Window & {
          __BUZZ_E2E_GET_RELAY_CONNECTION_STATE__?: () => string;
        }
      ).__BUZZ_E2E_GET_RELAY_CONNECTION_STATE__?.() === "connected",
  );
  const beforeReload = await page.evaluate(() => ({ ...localStorage }));
  for (const [key, value] of Object.entries(storage)) {
    if (/communities|identity|private.key|nsec/i.test(key))
      expect(beforeReload[key], key).toBe(value);
  }
  // The test bridge reseeds the launch timestamp on reload; compare retained
  // storage within each launch rather than comparing the fixture's timestamps.
  await page.reload();
  storage = await page.evaluate(() => ({ ...localStorage }));
  await expect(
    card.getByText("Upgrade Buzz to join this community"),
  ).toBeVisible();
  await expect(card).toContainText(
    "Download the latest Buzz client at buzz.xyz to continue.",
  );
  await card.getByRole("button", { name: "Download latest Buzz" }).click();
  expect(
    await page.evaluate(() =>
      (window as PolicyWindow).__BUZZ_E2E_COMMAND_PAYLOADS__
        ?.filter((call) => call.command === "plugin:opener|open_url")
        .map((call) => call.payload.url),
    ),
  ).toEqual(["https://buzz.xyz"]);
  await page.evaluate(() => {
    (window as PolicyWindow).__BUZZ_E2E__.mock.openerError =
      "No browser available";
  });
  await card.getByRole("button", { name: "Download latest Buzz" }).click();
  await expect(card.getByRole("alert")).toContainText("https://buzz.xyz");
  await card.getByRole("button", { name: "Change community address" }).click();
  await expect(card).toBeHidden();
  // Returning from address editing keeps the required verdict.
  await page.keyboard.press("Escape");
  await expect(
    card.getByText("Upgrade Buzz to join this community"),
  ).toBeVisible();
  await page.evaluate(() => {
    (window as PolicyWindow).__BUZZ_E2E__.mock.relayEntryPolicy = {
      required: false,
    };
  });
  await card.getByRole("button", { name: "Retry community check" }).click();
  await expect(card).toBeHidden();
  const after = await page.evaluate(() => ({ ...localStorage }));
  for (const [key, value] of Object.entries(storage)) {
    if (/communities|identity|private.key|nsec/i.test(key))
      expect(after[key], key).toBe(value);
  }
});

test("required community can switch to a saved free community and join another required host", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      relayEntryPolicy: {
        required: true,
        requiredRelay: "wss://required.example/",
      },
    },
    { skipCommunitySeed: true },
  );
  await page.addInitScript(() => {
    localStorage.setItem(
      "buzz-communities",
      JSON.stringify([
        {
          id: "required",
          name: "Required",
          relayUrl: "wss://required.example/",
          addedAt: new Date().toISOString(),
        },
        {
          id: "free",
          name: "Free",
          relayUrl: "ws://localhost:3000",
          addedAt: new Date().toISOString(),
        },
      ]),
    );
    localStorage.setItem("buzz-active-community-id", "required");
  });
  await page.goto("/");
  const card = page.getByTestId("relay-entry-guidance");
  await expect(
    card.getByText("Upgrade Buzz to join this community"),
  ).toBeVisible();
  await card.getByRole("button", { name: "Open Free", exact: true }).click();
  await expect(card).toBeHidden();
  const activeRail = page
    .getByRole("navigation", { name: "Communities" })
    .filter({
      has: page.locator(
        '[data-testid="community-rail-button-free"][aria-current="true"]',
      ),
    });
  await expect(activeRail).toBeVisible();
  await expect(page.getByTestId("community-rail-button-free")).toHaveCount(1);
  expect(
    await page.evaluate(
      () => JSON.parse(localStorage.getItem("buzz-communities") ?? "[]").length,
    ),
  ).toBe(2);
  // A new join must reach the same gate while the onboarding curtain is active.
  await page.evaluate(() => {
    (window as PolicyWindow).__BUZZ_E2E__.mock.relayEntryPolicy = {
      required: true,
      requiredRelay: "wss://new-required.example/",
    };
  });
  await activeRail
    .getByRole("button", { name: "Add community", exact: true })
    .click();
  await page.getByTestId("add-community-join").click();
  await page
    .getByLabel("Community URL or invite link")
    .fill("https://new-required.example");
  await page
    .getByRole("button", { name: "Join community", exact: true })
    .click();
  await expect(
    card.getByText("Upgrade Buzz to join this community"),
  ).toBeVisible();
  await expect(card).toContainText("new-required.example");
  await expect(card).toContainText(
    "Download the latest Buzz client at buzz.xyz to continue.",
  );
  await expect(
    card.getByRole("button", { name: "Download latest Buzz" }),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () => JSON.parse(localStorage.getItem("buzz-communities") ?? "[]").length,
    ),
  ).toBe(3);
  await card.getByRole("button", { name: "Open Free", exact: true }).click();
  await expect(card).toBeHidden();
  await expect(page.getByTestId("community-rail-button-free")).toHaveCount(1);
  expect(
    await page.evaluate(
      () => JSON.parse(localStorage.getItem("buzz-communities") ?? "[]").length,
    ),
  ).toBe(3);
});

test("changing a required join address cancels old onboarding and its held profile check", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      relayEntryPolicy: {
        required: true,
        requiredRelay: "wss://required.example",
      },
      deferProfileReads: true,
      profileHasEvent: true,
    },
    { skipCommunitySeed: true },
  );
  await page.addInitScript(() => {
    const now = new Date().toISOString();
    localStorage.setItem(
      "buzz-communities",
      JSON.stringify([
        {
          id: "required",
          name: "Required",
          relayUrl: "wss://required.example",
          addedAt: now,
        },
      ]),
    );
    localStorage.setItem("buzz-active-community-id", "required");
    localStorage.setItem(
      "buzz-community-onboarding-transaction.v1",
      JSON.stringify({
        id: "old-required-join",
        source: "add-community",
        stage: "connecting",
        relayUrl: "wss://required.example",
        communityName: "Required",
        communityId: "required",
        addedCommunity: true,
        createdAt: now,
        updatedAt: now,
      }),
    );
  });
  await page.routeWebSocket(
    /required\.example|replacement\.example/,
    (socket) => socket.close(),
  );
  await page.goto("/");
  const card = page.getByTestId("relay-entry-guidance");
  await expect(card).toBeVisible();
  await page.waitForFunction(
    () =>
      ((
        window as Window & {
          __BUZZ_E2E_PROFILE_READS_PENDING__?: () => number;
        }
      ).__BUZZ_E2E_PROFILE_READS_PENDING__?.() ?? 0) > 0,
  );
  // A label change keeps the live join; changing the relay cancels it.
  await card.getByRole("button", { name: "Change community address" }).click();
  const editor = page.getByTestId("community-change-overlay");
  await editor.locator("#community-edit-name").fill("Renamed");
  await editor.getByRole("button", { name: "Save changes" }).click();
  await editor.getByRole("button", { name: "Use anyway" }).click();
  await expect(card).toBeVisible();
  expect(
    await page.evaluate(
      () =>
        JSON.parse(
          localStorage.getItem("buzz-community-onboarding-transaction.v1") ??
            "null",
        )?.id,
    ),
  ).toBe("old-required-join");
  await card.getByRole("button", { name: "Change community address" }).click();
  await editor.locator("#community-edit-url").fill("wss://replacement.example");
  await editor.getByRole("button", { name: "Save changes" }).click();
  await editor.getByRole("button", { name: "Use anyway" }).click();
  await expect(card).toBeHidden();
  expect(
    await page.evaluate(() =>
      localStorage.getItem("buzz-community-onboarding-transaction.v1"),
    ),
  ).toBeNull();
  await page.evaluate(() => {
    (
      window as Window & {
        __BUZZ_E2E_RELEASE_PROFILE_READS__?: () => number;
      }
    ).__BUZZ_E2E_RELEASE_PROFILE_READS__?.();
  });
  await expect(
    page.getByRole("button", { name: "Search everything", exact: true }),
  ).toBeVisible();
  const state = await page.evaluate(() => ({ ...localStorage }));
  expect(state["buzz-communities"]).toContain("wss://replacement.example");
  expect(
    Object.keys(state).filter((key) =>
      key.startsWith(
        `buzz-community-onboarding-complete.v1:${encodeURIComponent("wss://required.example")}:`,
      ),
    ),
  ).toEqual([]);
  expect(state["buzz-community-onboarding-transaction.v1"]).toBeUndefined();
});
