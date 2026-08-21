import { expect, test, bootstrapE2ePage } from "../helpers/test";

import { installMockBridge } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";

test("settings sections share the Appearance rhythm", async ({ page }) => {
  await installMockBridge(page);
  await bootstrapE2ePage(page, "/");
  await openSettings(page, "appearance");

  const appearanceList = page
    .getByTestId("settings-theme")
    .locator('[data-slot="settings-section-list"]');
  const [referenceGap] = await appearanceList
    .locator(":scope > *")
    .evaluateAll((elements) =>
      elements.slice(1).map((element, index) => {
        const previous = elements[index];
        return (
          element.getBoundingClientRect().top -
          previous.getBoundingClientRect().bottom
        );
      }),
    );

  expect(referenceGap).toBeGreaterThan(0);

  for (const section of [
    "notifications",
    "voice",
    "agents",
    "shortcuts",
    "profile",
  ] as const) {
    await page.getByTestId(`settings-nav-${section}`).click();

    const list = page
      .getByTestId(`settings-${section}`)
      .locator('[data-slot="settings-section-list"]');
    await expect(list).toBeVisible();

    const gaps = await list.locator(":scope > *").evaluateAll((elements) =>
      elements.slice(1).map((element, index) => {
        const previous = elements[index];
        return (
          element.getBoundingClientRect().top -
          previous.getBoundingClientRect().bottom
        );
      }),
    );
    expect(gaps.length).toBeGreaterThan(0);
    expect(gaps.every((gap) => Math.abs(gap - referenceGap) <= 1)).toBe(true);
  }

  await page.getByTestId("settings-nav-agents").click();
  const headerToCardGap = async (testId: string) => {
    const group = page.getByTestId(testId);
    return group.evaluate((element) => {
      const lastHeaderLine = element.querySelector(
        '[data-slot="settings-section-header"] > div:first-child > :last-child',
      );
      const card = element.querySelector('[data-slot="settings-section-card"]');
      if (
        !(lastHeaderLine instanceof HTMLElement) ||
        !(card instanceof HTMLElement)
      ) {
        throw new Error("Missing section header or card");
      }
      return (
        card.getBoundingClientRect().top -
        lastHeaderLine.getBoundingClientRect().bottom
      );
    });
  };
  const [titleOnlyGap, subtitleGap] = await Promise.all([
    headerToCardGap("agents-preferences-card"),
    headerToCardGap("settings-harnesses"),
  ]);
  expect(titleOnlyGap).toBeGreaterThan(0);
  expect(Math.abs(titleOnlyGap - subtitleGap)).toBeLessThanOrEqual(1);
});

test("Profile sections keep visible cards and aligned actions", async ({
  page,
}) => {
  await installMockBridge(page);
  await bootstrapE2ePage(page, "/");
  await openSettings(page, "profile");

  const identity = page.getByTestId("profile-identity-card");
  const identityCard = identity.locator(
    'xpath=ancestor::*[@data-slot="settings-section-card"][1]',
  );
  await expect(identityCard).toBeVisible();
  await expect(
    page.getByText("Identity details", { exact: true }),
  ).toBeVisible();
  await expect(page.getByTestId("profile-identity-details")).toBeHidden();

  const signOut = page.getByTestId("settings-signout");
  const signOutCard = signOut.locator('[data-slot="settings-section-card"]');
  await expect(signOutCard).toBeVisible();
  await expect(
    signOutCard.getByRole("button", { name: "Delete my data" }),
  ).toBeVisible();
  await expect(signOut.getByText("Sign out", { exact: true })).toHaveCount(1);
  await expect(
    signOut.getByText("Sign out of Buzz", { exact: true }),
  ).toHaveCount(0);

  const profileInfo = page.getByTestId("profile-metadata-card");
  await profileInfo.scrollIntoViewIfNeeded();
  const [titleBottom, editBottom] = await Promise.all([
    profileInfo
      .getByRole("heading", { name: "Profile info" })
      .evaluate((element) => element.getBoundingClientRect().bottom),
    page
      .getByTestId("profile-metadata-edit")
      .evaluate((element) => element.getBoundingClientRect().bottom),
  ]);
  expect(Math.abs(titleBottom - editBottom)).toBeLessThanOrEqual(1);

  await page.getByTestId("settings-nav-updates").click();
  await expect(
    page
      .getByTestId("settings-updates")
      .getByText("Update status", { exact: true }),
  ).toHaveCount(1);
});
