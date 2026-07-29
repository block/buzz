import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// The fresh composer must target the default agent — the first managed
// (local) agent — rather than plain chat. Tab still cycles through every
// mode, including chat.

test("fresh composer defaults to an agent target, not chat", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.addInitScript(() => {
    localStorage.setItem("buzz.displayStyle", "developer");
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });

  const composer = page.getByTestId("dev-mode-composer");
  await composer.waitFor();

  // Mock managed agents (Fizz/Honey/Bumble) load async; the mode line must
  // settle on "to <agent>" without any Tab press.
  const modeLine = page.getByTestId("dev-mode-pill").first();
  await expect(modeLine).toContainText(/^to /);
  await expect(modeLine).not.toHaveText("chat");

  // Tab cycling still reaches chat mode and comes back around.
  await composer.focus();
  const initial = await modeLine.innerText();
  let sawChat = false;
  for (let step = 0; step < 12; step += 1) {
    await page.keyboard.press("Tab");
    const label = (await modeLine.innerText()).trim();
    if (label === "chat") sawChat = true;
    if (sawChat && label === initial.trim()) break;
  }
  expect(sawChat).toBe(true);
  await expect(modeLine).toHaveText(initial);
});

test("composer remembers the last Tab-cycled target across reloads", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.addInitScript(() => {
    localStorage.setItem("buzz.displayStyle", "developer");
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });

  const composer = page.getByTestId("dev-mode-composer");
  await composer.waitFor();

  const modeLine = page.getByTestId("dev-mode-pill").first();
  await expect(modeLine).toContainText(/^to /);
  const initial = (await modeLine.innerText()).trim();

  // Cycle to a different agent target (skipping plain chat).
  await composer.focus();
  let cycled = "";
  for (let step = 0; step < 12; step += 1) {
    await page.keyboard.press("Tab");
    const label = (await modeLine.innerText()).trim();
    if (label !== "chat" && label !== initial) {
      cycled = label;
      break;
    }
  }
  expect(cycled).not.toBe("");

  await page.reload({ waitUntil: "domcontentloaded" });
  await page.getByTestId("dev-mode-composer").waitFor();
  await expect(page.getByTestId("dev-mode-pill").first()).toHaveText(cycled);
});
