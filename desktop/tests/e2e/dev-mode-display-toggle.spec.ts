import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// ⇧⌘D toggles between the standard layout and dev mode. The open
// conversation survives the toggle in both directions: the dev shell seeds
// its state from the URL the standard layout left behind, and writes its
// own selection back into the URL as the user navigates within dev mode.

const GENERAL_CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const TOGGLE = "ControlOrMeta+Shift+D";

async function openStandardGeneral(page: import("@playwright/test").Page) {
  await installMockBridge(page);
  await page.goto(`/#/channels/${GENERAL_CHANNEL_ID}`, {
    waitUntil: "domcontentloaded",
  });
  await expect(page.getByTestId("chat-title")).toHaveText("general");
}

async function openDevMode(page: import("@playwright/test").Page) {
  await installMockBridge(page);
  await page.addInitScript(() => {
    localStorage.setItem("buzz.displayStyle", "developer");
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("dev-mode-composer").waitFor();
}

test("standard → dev → standard retains the open channel", async ({ page }) => {
  await openStandardGeneral(page);

  await page.keyboard.press(TOGGLE);
  await expect(page.getByTestId("dev-mode-topbar-channel")).toContainText(
    "general",
  );

  await page.keyboard.press(TOGGLE);
  await expect(page.getByTestId("chat-title")).toHaveText("general");
});

test("channel opened inside dev mode survives toggling to standard", async ({
  page,
}) => {
  await openDevMode(page);

  await page
    .getByTestId("dev-mode-channel-navigator")
    .getByText("# general", { exact: true })
    .click();
  await expect(page.getByTestId("dev-mode-topbar-channel")).toContainText(
    "general",
  );

  await page.keyboard.press(TOGGLE);
  await expect(page.getByTestId("chat-title")).toHaveText("general");
});

test("standard's open thread reopens as the dev side chat", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto(
    `/#/channels/${GENERAL_CHANNEL_ID}?thread=mock-general-welcome`,
    { waitUntil: "domcontentloaded" },
  );
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(page.getByTestId("message-thread-body")).toBeVisible();

  await page.keyboard.press(TOGGLE);
  const threadPanel = page.getByTestId("dev-mode-thread-panel");
  await expect(threadPanel).toBeVisible();
  await expect(threadPanel).toContainText("Welcome to #general");
});

test("dev side chat survives toggling as standard's thread panel", async ({
  page,
}) => {
  await openDevMode(page);

  await page
    .getByTestId("dev-mode-channel-navigator")
    .getByText("# general", { exact: true })
    .click();
  await page.getByTestId("dev-mode-transcript").waitFor();

  // Card selection is keyboard-only: ↑ selects the newest prompt card and
  // an empty-composer Enter opens its side chat.
  await page.getByTestId("dev-mode-composer").focus();
  await page.keyboard.press("ArrowUp");
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("dev-mode-thread-panel")).toBeVisible();

  // ↑ selected the newest prompt card — the reaction-target seed.
  const threadText = "React to me with a custom emoji";
  await expect(page.getByTestId("dev-mode-thread-panel")).toContainText(
    threadText,
  );

  await page.keyboard.press(TOGGLE);
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(page.getByTestId("message-thread-body")).toBeVisible();
  await expect(page.getByTestId("message-thread-head")).toContainText(
    threadText,
  );
});

test("peeking at the navigator without opening keeps the standard URL", async ({
  page,
}) => {
  await openStandardGeneral(page);

  await page.keyboard.press(TOGGLE);
  const composer = page.getByTestId("dev-mode-composer");
  await composer.waitFor();
  await expect(page.getByTestId("dev-mode-topbar-channel")).toContainText(
    "general",
  );

  await page.keyboard.press(TOGGLE);
  await expect(page).toHaveURL(
    new RegExp(`/channels/${GENERAL_CHANNEL_ID}(?:\\?|$)`),
  );
  await expect(page.getByTestId("chat-title")).toHaveText("general");
});
