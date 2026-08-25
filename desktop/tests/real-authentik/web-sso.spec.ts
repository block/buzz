import { expect, test, type Page } from "@playwright/test";

const AUTH_ORIGIN = "https://auth.bizfin.test";
const BUSINESS_ORIGIN = "https://business.bizfin.test";

async function signInToAuthentik(page: Page) {
  const username = process.env.POC_USER_USERNAME ?? "poc-user";
  const password = process.env.POC_USER_PASSWORD;
  if (!password) throw new Error("POC_USER_PASSWORD is required");

  await expect(page).toHaveURL(new RegExp(`^${AUTH_ORIGIN}`));
  const usernameInput = page.locator('input[name="uidField"]').first();
  await expect(usernameInput).toBeVisible();
  await usernameInput.fill(username);
  await page.getByRole("button", { name: /log in|continue/i }).click();

  const passwordInput = page.getByRole("textbox", { name: "Password" });
  await expect(passwordInput).toBeVisible();
  await passwordInput.fill(password);
  expect(
    await passwordInput.evaluate(
      (element) => (element as HTMLInputElement).value.length,
    ),
  ).toBeGreaterThan(0);
  await page.getByRole("button", { name: "Continue" }).click();
}

test("real Web dual-client SSO establishes an independent Business session", async ({
  context,
  page,
}) => {
  await page.goto("/?e2e=mock&resetDevState=1");
  await expect(page.getByTestId("workbench-auth-gate")).toBeVisible();

  await page.getByRole("button", { name: "Sign in with Authentik" }).click();
  await signInToAuthentik(page);
  await expect(page).toHaveURL(
    /^https:\/\/workbench\.bizfin\.test\/(?:\?e2e=mock)?$/,
  );
  await expect(page.getByTestId("workbench-auth-gate")).toBeHidden();

  const businessPage = await context.newPage();
  await businessPage.goto(`${BUSINESS_ORIGIN}/auth/login`);
  await expect(businessPage).toHaveURL(`${BUSINESS_ORIGIN}/`);
  await expect(businessPage.locator('input[name="uidField"]')).toHaveCount(0);
  await expect(businessPage.locator('input[name="password"]')).toHaveCount(0);
  await expect(businessPage.locator("#status")).toContainText(
    "Authenticated as POC User",
  );
  await expect(businessPage.locator("#claim-status")).toHaveText(
    "Groups claim verified",
  );

  const cookies = await context.cookies(BUSINESS_ORIGIN);
  const businessCookie = cookies.find(
    (candidate) => candidate.name === "business_session",
  );
  expect(businessCookie).toMatchObject({
    httpOnly: true,
    secure: true,
    sameSite: "Lax",
  });
  console.log("Business cookie metadata", {
    domain: businessCookie?.domain,
    hostOnly: !businessCookie?.domain.startsWith("."),
    httpOnly: businessCookie?.httpOnly,
    secure: businessCookie?.secure,
    sameSite: businessCookie?.sameSite,
    partitioned: "partitionKey" in (businessCookie ?? {}),
  });

  await page.getByTestId("business-dock-toggle").click();
  await expect(page.getByTestId("business-dock")).toBeVisible();
  await page
    .getByRole("button", {
      name: "Refresh business system",
    })
    .click();
  await expect(page.getByTestId("business-auth-debug")).toContainText(
    "POC User",
  );
  await expect(page.getByTestId("business-auth-debug")).toContainText(
    "Workbench groups: verified",
  );

  await page.getByTestId("business-dock-toggle").click();
  await expect(page.getByTestId("business-dock")).toBeHidden();
  await page.getByTestId("business-dock-toggle").click();
  await expect(page.getByTestId("business-auth-debug")).toContainText(
    "POC User",
  );

  await businessPage.reload();
  await expect(businessPage.locator("#status")).toContainText(
    "Authenticated as POC User",
  );

  await page.getByRole("button", { name: "Log out of Business" }).click();
  await expect(page.getByTestId("business-auth-required")).toBeVisible();
  const popupPromise = context.waitForEvent("page");
  await page.getByRole("button", { name: "Continue SSO" }).click();
  const popup = await popupPromise;
  await popup.waitForEvent("close");
  await expect(page.getByTestId("business-auth-debug")).toContainText(
    "POC User",
  );
});
