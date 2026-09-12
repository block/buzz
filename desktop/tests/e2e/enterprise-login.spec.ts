import { mkdir } from "node:fs/promises";
import { waitForAnimations } from "../helpers/animations";
import { expect, test } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";

test("corporate denial stays at login and never enters local-key onboarding", async ({
  page,
}) => {
  await page.addInitScript(() => {
    window.__BUZZ_E2E_ENTERPRISE__ = { enabled: true, loginFails: true };
  });
  await installMockBridge(page);
  await page.goto("/");
  const login = page.getByRole("button", {
    name: "Sign in with corporate account",
  });
  await expect(login).toBeVisible();
  await mkdir("test-results/enterprise-login", { recursive: true });
  await waitForAnimations(page);
  await page.screenshot({
    path: "test-results/enterprise-login/01-corporate-login.png",
  });
  await login.click();
  await expect(page.getByRole("alert")).toHaveText(
    "Sign-in did not complete. Try again.",
  );
  await expect(login).toBeEnabled();
  await waitForAnimations(page);
  await page.screenshot({
    path: "test-results/enterprise-login/02-denied-login.png",
  });
  const calls = await page.evaluate(() => window.__BUZZ_E2E_COMMANDS__ ?? []);
  expect(calls).not.toContain("get_nsec");
  expect(calls).not.toContain("persist_current_identity");
  expect(calls).not.toContain("sign_event");
});

test("successful corporate login installs community and bypasses key onboarding", async ({
  page,
}) => {
  await page.addInitScript(() => {
    window.__BUZZ_E2E_ENTERPRISE__ = { enabled: true };
  });
  await installMockBridge(page);
  await page.goto("/");
  await page
    .getByRole("button", { name: "Sign in with corporate account" })
    .click();
  await expect(
    page.getByRole("heading", { name: "Sign in to Buzz for work" }),
  ).not.toBeVisible();
  await expect(page.getByTestId("open-settings")).toBeVisible();
  await openSettings(page, "profile");
  const identity = page.getByTestId("profile-identity-card");
  if (
    !(await identity.evaluate(
      (element) => element instanceof HTMLDetailsElement && element.open,
    ))
  ) {
    await page.getByTestId("profile-identity-toggle").click();
  }
  await expect(
    page.getByText("Your organization holds your signing key.", {
      exact: false,
    }),
  ).toBeVisible();
  await expect(
    page.getByTestId("profile-private-key-toggle"),
  ).not.toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({
    path: "test-results/enterprise-login/03-managed-settings.png",
  });
  const calls = await page.evaluate(() => window.__BUZZ_E2E_COMMANDS__ ?? []);
  for (const command of [
    "get_nsec",
    "persist_current_identity",
    "nip44_encrypt_to_self",
    "nip44_decrypt_from_self",
    "create_managed_agent",
  ]) {
    expect(calls).not.toContain(command);
  }
  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: "Sign out of work account" }).click();
  await expect(
    page.getByRole("button", { name: "Sign in with corporate account" }),
  ).toBeVisible();
});
