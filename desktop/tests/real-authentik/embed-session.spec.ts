import { expect, test, type Page } from "@playwright/test";

const AUTH_ORIGIN = "https://auth.bizfin.localhost";
const BUSINESS_ORIGIN = "https://business.bizfin.localhost";

async function signInToAuthentik(page: Page) {
  const password = process.env.POC_USER_PASSWORD;
  if (!password) throw new Error("POC_USER_PASSWORD is required");
  await expect(page).toHaveURL(new RegExp(`^${AUTH_ORIGIN}`));
  const username = page.locator('input[name="uidField"]').first();
  await username.fill(process.env.POC_USER_USERNAME ?? "poc-user");
  await page.getByRole("button", { name: /log in|continue/i }).click();
  const passwordInput = page.getByRole("textbox", { name: "Password" });
  await passwordInput.fill(password);
  await page.getByRole("button", { name: "Continue" }).click();
}

test("one-time Embed Session is audience-bound and replay-safe", async ({
  browser,
  page,
}) => {
  await page.goto(`${BUSINESS_ORIGIN}/auth/login`);
  await signInToAuthentik(page);
  await expect(page).toHaveURL(`${BUSINESS_ORIGIN}/`);
  await expect(page.locator("#status")).toContainText(
    "Authenticated as POC User",
  );
  await expect(page.locator("#claim-status")).toHaveText(
    "Groups claim verified",
  );

  const issuance = await page.evaluate(async () => {
    const response = await fetch("/api/embed-sessions", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ target: "/embed/orders/POC-1" }),
    });
    return { status: response.status, body: await response.json() };
  });
  expect(issuance.status).toBe(201);
  const body = issuance.body as {
    id: string;
    embedUrl: string;
    expiresIn: number;
  };
  expect(body.expiresIn).toBe(30);
  expect(new URL(body.embedUrl).origin).toBe(BUSINESS_ORIGIN);

  const invalidTargetStatus = await page.evaluate(
    async () =>
      (
        await fetch("/api/embed-sessions", {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ target: "https://evil.example/embed/no" }),
        })
      ).status,
  );
  expect(invalidTargetStatus).toBe(400);

  const webviewContext = await browser.newContext({ ignoreHTTPSErrors: true });
  const webview = await webviewContext.newPage();
  await webview.goto(body.embedUrl);
  await expect(webview).toHaveURL(`${BUSINESS_ORIGIN}/embed/orders/POC-1`);
  await expect(webview.locator("#status")).toContainText(
    "Authenticated as POC User",
  );
  await expect(webview.locator("#claim-status")).toHaveText(
    "Groups claim verified",
  );

  const webviewCookies = await webviewContext.cookies(BUSINESS_ORIGIN);
  expect(
    webviewCookies.find((cookie) => cookie.name === "business_session"),
  ).toMatchObject({ httpOnly: true, secure: true, sameSite: "None" });

  const replay = await webview.goto(body.embedUrl);
  expect(replay?.status()).toBe(410);
  expect(replay?.headers()["cache-control"]).toBe("no-store");
  expect(replay?.headers()["referrer-policy"]).toBe("no-referrer");
  await expect(
    webview.getByRole("heading", { name: "Embed session unavailable" }),
  ).toBeVisible();

  const invalid = await webview.goto(
    `${BUSINESS_ORIGIN}/embed/bootstrap?code=${"x".repeat(43)}`,
  );
  expect(invalid?.status()).toBe(404);
  expect(invalid?.headers()["cache-control"]).toBe("no-store");
  expect(invalid?.headers()["referrer-policy"]).toBe("no-referrer");

  const revoked = await page.evaluate(async () => {
    const issued = await fetch("/api/embed-sessions", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ target: "/embed/orders/POC-2" }),
    });
    const ticket = await issued.json();
    const response = await fetch(`/api/embed-sessions/${ticket.id}/revoke`, {
      method: "POST",
    });
    return { embedUrl: ticket.embedUrl, status: response.status };
  });
  expect(revoked.status).toBe(204);
  const revokedResponse = await webview.goto(revoked.embedUrl);
  expect(revokedResponse?.status()).toBe(410);

  const unauthenticatedRevoke = await webview.request.post(
    `${BUSINESS_ORIGIN}/api/embed-sessions/${body.id}/revoke`,
  );
  expect(unauthenticatedRevoke.status()).toBe(404);
  await webviewContext.close();
});
