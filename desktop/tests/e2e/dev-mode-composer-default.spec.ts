import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// The fresh composer must target the default agent — the first managed
// (local) agent — rather than plain chat. Tab toggles chat ↔ the last agent;
// ⌃Tab cycles through the agents only. Relay agents need an explicit
// allowlist to appear in the cycle, so the specs seed managed agents
// (always eligible).

const managedAgents = [
  {
    pubkey: "1111111111111111111111111111111111111111111111111111111111111111",
    name: "Fizz",
    status: "stopped" as const,
  },
  {
    pubkey: "2222222222222222222222222222222222222222222222222222222222222222",
    name: "Honey",
    status: "stopped" as const,
  },
];

test("fresh composer defaults to an agent target, not chat", async ({
  page,
}) => {
  await installMockBridge(page, { managedAgents });
  await page.addInitScript(() => {
    localStorage.setItem("buzz.displayStyle", "developer");
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });

  const composer = page.getByTestId("dev-mode-composer");
  await composer.waitFor();

  // Seeded managed agents load async; the mode line must settle on
  // "to <agent>" without any Tab press.
  const modeLine = page.getByTestId("dev-mode-pill").first();
  await expect(modeLine).toContainText(/^to /);
  await expect(modeLine).not.toHaveText("chat");

  // Tab toggles to plain chat and straight back to the same agent.
  await composer.focus();
  const initial = await modeLine.innerText();
  await page.keyboard.press("Tab");
  await expect(modeLine).toHaveText("chat");
  await page.keyboard.press("Tab");
  await expect(modeLine).toHaveText(initial);
});

test("⌃Tab cycles through agents only, skipping chat", async ({ page }) => {
  await installMockBridge(page, { managedAgents });
  await page.addInitScript(() => {
    localStorage.setItem("buzz.displayStyle", "developer");
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });

  const composer = page.getByTestId("dev-mode-composer");
  await composer.waitFor();

  const modeLine = page.getByTestId("dev-mode-pill").first();
  await expect(modeLine).toHaveText("to Fizz");

  await composer.focus();
  await page.keyboard.press("Control+Tab");
  await expect(modeLine).toHaveText("to Honey");

  // Wraps around the agent list without ever landing on chat.
  await page.keyboard.press("Control+Tab");
  await expect(modeLine).toHaveText("to Fizz");

  // ⌃⇧Tab reverses.
  await page.keyboard.press("Control+Shift+Tab");
  await expect(modeLine).toHaveText("to Honey");

  // From chat, ⌃Tab resumes at the last agent instead of restarting.
  await page.keyboard.press("Tab");
  await expect(modeLine).toHaveText("chat");
  await page.keyboard.press("Control+Tab");
  await expect(modeLine).toHaveText("to Honey");
});

test("Tab from chat returns to the last agent, not the default", async ({
  page,
}) => {
  await installMockBridge(page, { managedAgents });
  await page.addInitScript(() => {
    localStorage.setItem("buzz.displayStyle", "developer");
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });

  const composer = page.getByTestId("dev-mode-composer");
  await composer.waitFor();

  const modeLine = page.getByTestId("dev-mode-pill").first();
  await expect(modeLine).toHaveText("to Fizz");

  await composer.focus();
  await page.keyboard.press("Control+Tab");
  await expect(modeLine).toHaveText("to Honey");

  await page.keyboard.press("Tab");
  await expect(modeLine).toHaveText("chat");
  await page.keyboard.press("Tab");
  await expect(modeLine).toHaveText("to Honey");
});

test("composer remembers the last cycled target across reloads", async ({
  page,
}) => {
  await installMockBridge(page, { managedAgents });
  await page.addInitScript(() => {
    localStorage.setItem("buzz.displayStyle", "developer");
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });

  const composer = page.getByTestId("dev-mode-composer");
  await composer.waitFor();

  const modeLine = page.getByTestId("dev-mode-pill").first();
  await expect(modeLine).toHaveText("to Fizz");

  // Cycle to a different agent target.
  await composer.focus();
  await page.keyboard.press("Control+Tab");
  await expect(modeLine).toHaveText("to Honey");

  await page.reload({ waitUntil: "domcontentloaded" });
  await page.getByTestId("dev-mode-composer").waitFor();
  await expect(page.getByTestId("dev-mode-pill").first()).toHaveText(
    "to Honey",
  );
});
