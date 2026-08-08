import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { expect, test } from "@playwright/test";

const ROUTER_RS = fileURLToPath(
  new URL("../../crates/buzz-relay/src/router.rs", import.meta.url),
);

/// The exact policy the relay serves on admin SPA documents. Read from the
/// relay source rather than copied, so this test can never pass against a
/// policy operators do not actually get. The preview server used here serves
/// no CSP of its own, so it is injected below.
function adminCsp() {
  const source = readFileSync(ROUTER_RS, "utf8");
  const match = source.match(/const ADMIN_CSP: &str = "([^"]+)";/);
  if (!match) throw new Error(`ADMIN_CSP not found in ${ROUTER_RS}`);
  return match[1];
}

const TOKEN =
  "5f0e1d2c3b4a59687786958493a2b1c0decadebeefcafe0123456789abcdef01";

test("the relay admin csp does not break the built dashboard", async ({
  page,
}) => {
  const csp = adminCsp();
  expect(csp).toContain("frame-ancestors 'none'");
  expect(csp).not.toContain("unsafe-inline");

  await page.addInitScript((token) => {
    sessionStorage.setItem("buzz-admin-token", token);
  }, TOKEN);
  await page.route("**/api/admin/v1/**", (route) =>
    route.fulfill({ contentType: "application/json", body: "[]" }),
  );
  // Only the document request: the API call to /reports carries a query string.
  await page.route(
    (url) => url.pathname === "/reports" && url.search === "",
    async (route) => {
      const response = await route.fetch();
      await route.fulfill({
        response,
        headers: { ...response.headers(), "content-security-policy": csp },
      });
    },
  );

  const violations: string[] = [];
  page.on("console", (message) => {
    if (message.text().includes("Content Security Policy"))
      violations.push(message.text());
  });

  await page.goto("/reports");

  // Rendering at all proves the bundle's script and stylesheet loaded, and the
  // empty state proves the fetch to /api/admin/v1 survived `connect-src 'self'`.
  await expect(
    page.getByRole("heading", { name: "Open reports" }),
  ).toBeVisible();
  await expect(page.getByText("No records.")).toBeVisible();
  expect(violations).toEqual([]);
});

test("the linked favicon loads under the admin csp", async ({ page }) => {
  const csp = adminCsp();

  await page.route(
    (url) => url.pathname === "/" && url.search === "",
    async (route) => {
      const response = await route.fetch();
      await route.fulfill({
        response,
        headers: { ...response.headers(), "content-security-policy": csp },
      });
    },
  );

  const violations: string[] = [];
  page.on("console", (message) => {
    if (message.text().includes("Content Security Policy"))
      violations.push(message.text());
  });

  await page.goto("/");

  const href = await page.locator("link[rel=icon]").getAttribute("href");
  expect(href).toBe("/favicon.svg");

  // Headless Chromium never issues the `<link rel=icon>` request itself, so
  // load the same file the same way the policy sees it: an image fetch under
  // `img-src 'self'`. A blocked fetch rejects `decode()`; a successful one
  // proves the icon renders, and that the SVG's own inline <style> is not
  // subject to the embedding document's `style-src`.
  const decoded = await page.evaluate(async (src) => {
    const image = new Image();
    image.src = src;
    try {
      await image.decode();
      return { ok: true, width: image.naturalWidth };
    } catch (error) {
      return { ok: false, error: String(error) };
    }
  }, href as string);

  expect(decoded.ok).toBe(true);
  expect(decoded.width).toBeGreaterThan(0);
  expect(violations).toEqual([]);
});
