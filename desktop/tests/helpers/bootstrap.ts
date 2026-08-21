import type { Page } from "@playwright/test";

export const E2E_APP_ORIGIN = "http://127.0.0.1:4173";

type BootstrapE2ePageOptions = {
  /** Relative route to load after all test setup has been registered. */
  path?: string;
  /** Test setup such as bridge or storage init-script registration. */
  beforeNavigate?: () => Promise<void>;
  expectedOrigin?: string;
};

/**
 * Registers page setup and commits the first app navigation before a spec uses
 * browser storage or page evaluation. This makes an unavailable preview server
 * fail at its source instead of later as an opaque-origin storage error.
 */
export async function bootstrapE2ePage(
  page: Page,
  pathOrOptions: string | BootstrapE2ePageOptions = {},
  gotoOptions?: Parameters<Page["goto"]>[1],
): Promise<void> {
  const {
    path = "/",
    beforeNavigate,
    expectedOrigin = E2E_APP_ORIGIN,
  } = typeof pathOrOptions === "string"
    ? { path: pathOrOptions }
    : pathOrOptions;
  await beforeNavigate?.();

  const target = new URL(path, expectedOrigin).toString();
  let response: Awaited<ReturnType<Page["goto"]>>;
  try {
    response = await page.goto(target, gotoOptions);
  } catch (error) {
    throw bootstrapFailure(expectedOrigin, page.url(), error);
  }

  if (response && !response.ok()) {
    throw bootstrapFailure(
      expectedOrigin,
      page.url(),
      `navigation returned HTTP ${response.status()}`,
    );
  }

  if (new URL(page.url()).origin !== expectedOrigin) {
    throw bootstrapFailure(
      expectedOrigin,
      page.url(),
      "navigation changed origin",
    );
  }
}

function bootstrapFailure(
  expectedOrigin: string,
  currentUrl: string,
  cause: unknown,
): Error {
  return new Error(
    `E2E app navigation did not commit to ${expectedOrigin} (current URL: ${currentUrl}). ` +
      `Run pnpm test:e2e:smoke or run pnpm build:e2e and check the preview server. Cause: ${String(cause)}`,
  );
}
