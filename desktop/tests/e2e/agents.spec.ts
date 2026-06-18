import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

async function gotoApp(page: import("@playwright/test").Page) {
  let lastError: unknown = null;

  for (const attempt of [0, 1]) {
    await page.goto("/", { waitUntil: "domcontentloaded" });
    await waitForInvokeBridge(page);

    try {
      await expect(page.getByTestId("open-agents-view")).toBeVisible({
        timeout: 10_000,
      });
      return;
    } catch (error) {
      lastError = error;
      if (attempt === 1) {
        throw error;
      }
    }
  }

  throw lastError;
}

async function openPersonaCatalog(page: import("@playwright/test").Page) {
  await page
    .getByTestId("agents-library-personas")
    .getByRole("button", { name: "New", exact: true })
    .click();
  await page.getByText("Choose from Catalog...").click();
}

async function getCatalogOrder(page: import("@playwright/test").Page) {
  return page
    .locator('[data-testid^="persona-catalog-card-target-"]')
    .evaluateAll((elements) =>
      elements.map((element) => element.getAttribute("data-testid") ?? ""),
    );
}

async function waitForInvokeBridge(page: import("@playwright/test").Page) {
  await page.waitForFunction(
    () => {
      const tauriWindow = window as Window & {
        __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: unknown;
        __TAURI_INTERNALS__?: {
          invoke?: unknown;
        };
      };

      return (
        typeof tauriWindow.__BUZZ_E2E_INVOKE_MOCK_COMMAND__ === "function" ||
        typeof tauriWindow.__TAURI_INTERNALS__?.invoke === "function"
      );
    },
    null,
    { timeout: 5_000 },
  );
}

async function invokeTauri<T>(
  page: import("@playwright/test").Page,
  command: string,
  payload?: Record<string, unknown>,
): Promise<T> {
  await waitForInvokeBridge(page);

  return page.evaluate(
    async ({ command: targetCommand, payload: targetPayload }) => {
      const tauriWindow = window as Window & {
        __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
          command: string,
          payload?: Record<string, unknown>,
        ) => Promise<unknown>;
        __TAURI_INTERNALS__?: {
          invoke?: (
            command: string,
            payload?: Record<string, unknown>,
          ) => Promise<unknown>;
        };
      };

      const invoke =
        tauriWindow.__BUZZ_E2E_INVOKE_MOCK_COMMAND__ ??
        tauriWindow.__TAURI_INTERNALS__?.invoke;
      if (!invoke) {
        throw new Error("Mock invoke bridge is unavailable.");
      }

      return (await invoke(targetCommand, targetPayload)) as T;
    },
    { command, payload },
  );
}

async function invokeTauriExpectError(
  page: import("@playwright/test").Page,
  command: string,
  payload?: Record<string, unknown>,
) {
  await waitForInvokeBridge(page);

  return page.evaluate(
    async ({ command: targetCommand, payload: targetPayload }) => {
      const tauriWindow = window as Window & {
        __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
          command: string,
          payload?: Record<string, unknown>,
        ) => Promise<unknown>;
        __TAURI_INTERNALS__?: {
          invoke?: (
            command: string,
            payload?: Record<string, unknown>,
          ) => Promise<unknown>;
        };
      };

      const invoke =
        tauriWindow.__BUZZ_E2E_INVOKE_MOCK_COMMAND__ ??
        tauriWindow.__TAURI_INTERNALS__?.invoke;
      if (!invoke) {
        throw new Error("Mock invoke bridge is unavailable.");
      }

      try {
        await invoke(targetCommand, targetPayload);
        return null;
      } catch (error) {
        return error instanceof Error ? error.message : String(error);
      }
    },
    { command, payload },
  );
}

test("built-in personas are chosen from the dialog and can be selected", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 420 });
  await gotoApp(page);
  await page.getByTestId("open-agents-view").click();

  await expect(page.getByTestId("agents-library-personas")).toBeVisible();
  await openPersonaCatalog(page);
  await expect(page.getByTestId("persona-catalog-dialog")).toContainText(
    "Fizz",
  );
  await expect(page.getByTestId("persona-catalog-dialog-header")).toBeVisible();
  await expect(
    page.getByTestId("persona-catalog-dialog-scroll-area"),
  ).toBeVisible();
  await expect(
    page.getByTestId("persona-catalog-dialog-scroll-area"),
  ).toHaveCSS("overflow-y", "auto");
  expect(
    await page
      .getByTestId("persona-catalog-dialog-scroll-area")
      .evaluate(
        (element) =>
          element.scrollHeight > element.clientHeight &&
          element.clientHeight > 0,
      ),
  ).toBe(true);
  await expect(page.getByTestId("persona-catalog-dialog-footer")).toBeVisible();
  await expect(page.getByRole("tooltip")).toHaveCount(0);
  const initialCatalogOrder = await getCatalogOrder(page);

  await page.getByTestId("persona-catalog-card-target-builtin:fizz").click();
  await expect(
    page
      .locator("[data-sonner-toast]")
      .filter({ hasText: "Selected Fizz for My Agents." }),
  ).toBeVisible();

  await expect(page.getByTestId("agents-library-personas")).toContainText(
    "Fizz",
  );
  await expect(
    page.getByTestId("persona-catalog-card-target-builtin:fizz"),
  ).toHaveAttribute("aria-pressed", "true");
  await expect.poll(() => getCatalogOrder(page)).toEqual(initialCatalogOrder);

  await page.getByTestId("persona-catalog-card-target-builtin:fizz").click();
  await expect(
    page
      .locator("[data-sonner-toast]")
      .filter({ hasText: "Deselected Fizz from My Agents." }),
  ).toBeVisible();
  await expect(
    page.getByTestId("persona-catalog-card-target-builtin:fizz"),
  ).toHaveAttribute("aria-pressed", "false");
  await expect.poll(() => getCatalogOrder(page)).toEqual(initialCatalogOrder);
});

test("persona catalog can reopen from the populated library header", async ({
  page,
}) => {
  await gotoApp(page);
  await page.getByTestId("open-agents-view").click();
  await openPersonaCatalog(page);

  await page.getByTestId("persona-catalog-card-target-builtin:fizz").click();
  await expect(page.getByTestId("agents-library-personas")).toContainText(
    "Fizz",
  );

  await page.getByTestId("persona-catalog-dialog-done").click();
  await openPersonaCatalog(page);

  await expect(page.getByTestId("persona-catalog-dialog")).toBeVisible();
  await expect(
    page.getByTestId("persona-catalog-card-target-builtin:fizz"),
  ).toHaveAttribute("aria-pressed", "true");
});

test("persona catalog chooser order stays stable when selection changes", async ({
  page,
}) => {
  await gotoApp(page);
  await page.getByTestId("open-agents-view").click();
  await openPersonaCatalog(page);

  const before = await getCatalogOrder(page);

  await page.getByTestId("persona-catalog-card-target-builtin:fizz").click();
  await expect(
    page
      .locator("[data-sonner-toast]")
      .filter({ hasText: "Selected Fizz for My Agents." }),
  ).toBeVisible();

  expect(await getCatalogOrder(page)).toEqual(before);
});

test("catalog details sheet shows the full persona details", async ({
  page,
}) => {
  await gotoApp(page);
  await page.getByTestId("open-agents-view").click();
  await openPersonaCatalog(page);

  await page.getByTestId("persona-catalog-details-builtin:fizz").click();
  const detailSelectionTarget = page.getByTestId(
    "persona-catalog-detail-selection-target-builtin:fizz",
  );

  await expect(page.getByTestId("persona-catalog-details-sheet")).toContainText(
    "Fizz",
  );
  await expect(page.getByTestId("persona-catalog-details-sheet")).toContainText(
    "You are Fizz.",
  );
  await expect(
    page.getByTestId("persona-catalog-detail-selection-title"),
  ).toHaveText("Available in Persona Catalog");
  await expect(detailSelectionTarget).toHaveAttribute(
    "aria-label",
    "Select Fizz in My Agents",
  );
  await expect(detailSelectionTarget).toHaveAttribute("aria-pressed", "false");

  await detailSelectionTarget.click();
  await expect(
    page.getByTestId("persona-catalog-detail-selection-title"),
  ).toHaveText("Selected for My Agents");
  await expect(detailSelectionTarget).toHaveAttribute(
    "aria-label",
    "Deselect Fizz in My Agents",
  );
  await expect(detailSelectionTarget).toHaveAttribute("aria-pressed", "true");
  await expect(page.getByTestId("agents-library-personas")).toContainText(
    "Fizz",
  );
});

test("inactive built-ins cannot be used to create teams", async ({ page }) => {
  await gotoApp(page);

  const error = await invokeTauriExpectError(page, "create_team", {
    input: {
      name: "Fizzes",
      personaIds: ["builtin:fizz"],
    },
  });

  expect(error).toBe(
    "Fizz is not in My Agents. Choose it from Persona Catalog first.",
  );
});

test("built-in deselection failures show up in Persona Catalog", async ({
  page,
}) => {
  await gotoApp(page);

  await page.getByTestId("open-agents-view").click();
  await openPersonaCatalog(page);
  await page.getByTestId("persona-catalog-card-target-builtin:fizz").click();

  await invokeTauri(page, "create_team", {
    input: {
      name: "Fizzes",
      personaIds: ["builtin:fizz"],
    },
  });

  await page.getByTestId("persona-catalog-card-target-builtin:fizz").click();

  await expect(
    page
      .locator("[data-sonner-toast]")
      .filter({ hasText: "Fizz is still referenced by a team." }),
  ).toBeVisible();
});

test("personas referenced by teams cannot be deleted", async ({ page }) => {
  await gotoApp(page);

  const persona = await invokeTauri<{ id: string }>(page, "create_persona", {
    input: {
      displayName: "Analyst",
      systemPrompt: "You are Analyst.",
    },
  });

  await invokeTauri(page, "create_team", {
    input: {
      name: "Analysts",
      personaIds: [persona.id],
    },
  });

  const error = await invokeTauriExpectError(page, "delete_persona", {
    id: persona.id,
  });

  expect(error).toBe(
    "Analyst is still referenced by a team. Remove it from those teams first.",
  );
});
