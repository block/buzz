import { expect, test, bootstrapE2ePage } from "../helpers/test";
import { E2E_APP_ORIGIN } from "../helpers/bootstrap";

test.beforeEach(async ({ page }) => {
  await bootstrapE2ePage(page);
});

test("starts each test at the app origin after explicit bootstrap", async ({
  page,
}) => {
  expect(new URL(page.url()).origin).toBe(E2E_APP_ORIGIN);
});

test("reports a failed initial navigation at its source", async ({ page }) => {
  await expect(
    bootstrapE2ePage(page, { expectedOrigin: "http://127.0.0.1:1" }),
  ).rejects.toThrow(
    /E2E app navigation did not commit to http:\/\/127\.0\.0\.1:1 \(current URL: http:\/\/127\.0\.0\.1:4173\/\).*Run pnpm test:e2e:smoke or run pnpm build:e2e and check the preview server/,
  );
});
