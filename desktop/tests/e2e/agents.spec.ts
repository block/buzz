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
  await page.getByTestId("new-agent-card").click();
  await page.getByRole("menuitem", { name: "Choose from catalog" }).click();
}

async function getCatalogOrder(page: import("@playwright/test").Page) {
  return page
    .locator('[data-testid^="persona-catalog-list-item-"]')
    .evaluateAll((elements) =>
      elements.map((element) => element.getAttribute("data-testid") ?? ""),
    );
}

async function selectCatalogPersona(
  page: import("@playwright/test").Page,
  personaId: string,
) {
  await page.getByTestId(`persona-catalog-list-item-${personaId}`).click();
}

async function useCatalogPersona(
  page: import("@playwright/test").Page,
  personaId: string,
) {
  await page
    .getByTestId(`persona-catalog-use-agent-target-${personaId}`)
    .click();
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

test("built-in personas are used from the catalog dialog", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 420 });
  await gotoApp(page);
  await page.getByTestId("open-agents-view").click();

  await expect(page.getByTestId("agents-library-personas")).toBeVisible();
  await openPersonaCatalog(page);
  await expect(page.getByTestId("persona-catalog-dialog")).toContainText(
    "Fizz",
  );
  const previewPersonas = [
    ["builtin:product-strategist", "Product Strategist"],
    ["builtin:implementation-partner", "Implementation Partner"],
    ["builtin:qa-reviewer", "QA Reviewer"],
    ["builtin:work-coordinator", "Work Coordinator"],
    ["builtin:support-guide", "Support Guide"],
    ["builtin:experiment-designer", "Experiment Designer"],
  ] as const;
  for (const [, personaName] of previewPersonas) {
    await expect(page.getByTestId("persona-catalog-dialog")).toContainText(
      personaName,
    );
  }
  for (const [personaId, personaName] of previewPersonas) {
    await expect(
      page
        .getByTestId(`persona-catalog-list-item-${personaId}`)
        .getByRole("img", { name: `${personaName} avatar` }),
    ).toHaveAttribute("src", /.+/);
  }
  await expect(page.getByTestId("persona-catalog-dialog-header")).toBeVisible();
  await expect(
    page.getByTestId("persona-catalog-dialog-scroll-area"),
  ).toBeVisible();
  await expect(
    page.getByTestId("persona-catalog-dialog-scroll-area"),
  ).toHaveCSS("overflow-y", "auto");
  const catalogScrollAreaMetrics = await page
    .getByTestId("persona-catalog-dialog-scroll-area")
    .evaluate((element) => ({
      clientHeight: element.clientHeight,
      scrollHeight: element.scrollHeight,
    }));
  expect(catalogScrollAreaMetrics.clientHeight).toBeGreaterThan(0);
  expect(catalogScrollAreaMetrics.scrollHeight).toBeGreaterThanOrEqual(
    catalogScrollAreaMetrics.clientHeight,
  );
  await expect(page.getByTestId("persona-catalog-dialog-body")).toBeVisible();
  await expect(page.getByTestId("persona-catalog-dialog")).not.toContainText(
    "Done",
  );
  await expect(page.getByRole("tooltip")).toHaveCount(0);
  const initialCatalogOrder = await getCatalogOrder(page);

  await selectCatalogPersona(page, "builtin:fizz");
  await useCatalogPersona(page, "builtin:fizz");
  await expect(
    page
      .locator("[data-sonner-toast]")
      .filter({ hasText: "Selected Fizz for My Agents." }),
  ).toBeVisible();

  await expect(page.getByTestId("agents-library-personas")).toContainText(
    "Fizz",
  );
  await expect(
    page.getByTestId("persona-catalog-use-agent-target-builtin:fizz"),
  ).toHaveText("Added to My Agents");
  await expect(
    page.getByTestId("persona-catalog-use-agent-target-builtin:fizz"),
  ).toBeDisabled();
  await expect(page.getByTestId("persona-catalog-dialog")).not.toContainText(
    "Remove from My Agents",
  );
  await expect.poll(() => getCatalogOrder(page)).toEqual(initialCatalogOrder);
});

test("agent catalog can reopen from the populated library header", async ({
  page,
}) => {
  await gotoApp(page);
  await page.getByTestId("open-agents-view").click();
  await openPersonaCatalog(page);

  await selectCatalogPersona(page, "builtin:fizz");
  await useCatalogPersona(page, "builtin:fizz");
  await expect(page.getByTestId("agents-library-personas")).toContainText(
    "Fizz",
  );

  await page.keyboard.press("Escape");
  await openPersonaCatalog(page);

  await expect(page.getByTestId("persona-catalog-dialog")).toBeVisible();
  await selectCatalogPersona(page, "builtin:fizz");
  await expect(
    page.getByTestId("persona-catalog-use-agent-target-builtin:fizz"),
  ).toBeDisabled();
});

test("agent catalog chooser order stays stable when selection changes", async ({
  page,
}) => {
  await gotoApp(page);
  await page.getByTestId("open-agents-view").click();
  await openPersonaCatalog(page);

  const before = await getCatalogOrder(page);

  await selectCatalogPersona(page, "builtin:fizz");
  await useCatalogPersona(page, "builtin:fizz");
  await expect(
    page
      .locator("[data-sonner-toast]")
      .filter({ hasText: "Selected Fizz for My Agents." }),
  ).toBeVisible();

  expect(await getCatalogOrder(page)).toEqual(before);
});

test("catalog detail pane shows the full persona details", async ({ page }) => {
  await gotoApp(page);
  await page.getByTestId("open-agents-view").click();
  await openPersonaCatalog(page);

  await selectCatalogPersona(page, "builtin:fizz");
  const useAgentTarget = page.getByTestId(
    "persona-catalog-use-agent-target-builtin:fizz",
  );

  await expect(page.getByTestId("persona-catalog-detail-pane")).toContainText(
    "Fizz",
  );
  await expect(
    page.getByTestId("persona-catalog-detail-pane"),
  ).not.toContainText("Added by You");
  await expect(page.getByTestId("persona-catalog-detail-pane")).toContainText(
    "You are Fizz.",
  );
  await expect(page.getByTestId("persona-catalog-detail-pane")).toContainText(
    "Built-in persona",
  );
  await expect(page.getByTestId("persona-catalog-detail-pane")).toContainText(
    "Preferred model",
  );
  await expect(page.getByTestId("persona-catalog-detail-pane")).toContainText(
    "Preferred runtime",
  );
  await expect(page.getByTestId("persona-catalog-detail-pane")).toContainText(
    "Agent instruction",
  );
  await expect(useAgentTarget).toHaveAttribute(
    "aria-label",
    "Add Fizz from Agent Catalog",
  );
  await expect(useAgentTarget).toHaveText("Add agent");

  await useAgentTarget.click();
  await expect(page.getByTestId("agents-library-personas")).toContainText(
    "Fizz",
  );
});

test("custom personas can be shown in the agent catalog", async ({ page }) => {
  const analystPersonaId = "custom:analyst";
  await installMockBridge(page, {
    personas: [
      {
        id: analystPersonaId,
        displayName: "Analyst",
        systemPrompt: "You are Analyst.",
      },
    ],
  });
  await gotoApp(page);

  await page.getByTestId("open-agents-view").click();
  await expect(page.getByTestId("agents-library-personas")).toContainText(
    "Analyst",
  );

  await page.getByLabel("Open actions for Analyst").click();
  await expect(page.getByRole("menuitem")).toHaveText([
    "Catalog options",
    "Edit",
    "Duplicate",
    "Remove from My Agents",
  ]);
  await expect(
    page.getByRole("menuitem", { name: "Catalog options" }),
  ).toBeVisible();
  await expect(page.getByRole("menuitem", { name: "Export" })).toHaveCount(0);
  await page.getByRole("menuitem", { name: "Catalog options" }).click();

  await expect(page.getByTestId("persona-share-dialog")).toBeVisible();
  await expect(page.getByTestId("persona-share-dialog")).toContainText(
    "Catalog options",
  );
  await expect(page.getByTestId("persona-share-dialog")).toContainText(
    "Added by You",
  );
  await expect(page.getByTestId("persona-share-copy-link")).toHaveCount(0);
  await expect(page.getByText("Show in my catalog")).toBeVisible();
  await expect(page.getByTestId("persona-share-export")).toBeVisible();
  await page.getByTestId("persona-share-show-in-catalog").click();
  await page.keyboard.press("Escape");

  await openPersonaCatalog(page);
  await expect(
    page.getByTestId(`persona-catalog-list-item-${analystPersonaId}`),
  ).toContainText("Analyst");
  await selectCatalogPersona(page, analystPersonaId);
  await expect(page.getByTestId("persona-catalog-detail-pane")).toContainText(
    "Custom persona",
  );
  await expect(
    page.getByTestId(`persona-catalog-use-agent-target-${analystPersonaId}`),
  ).toHaveText("Added to My Agents");
});

test("team-managed personas do not expose editable actions", async ({
  page,
}) => {
  await installMockBridge(page, {
    personas: [
      {
        id: "team:analyst",
        displayName: "Team Analyst",
        sourceTeam: "team-research-002",
        systemPrompt: "You are Team Analyst.",
      },
    ],
  });
  await gotoApp(page);

  await page.getByTestId("open-agents-view").click();
  await page.getByLabel("Open actions for Team Analyst").click();

  await expect(page.getByRole("menuitem")).toHaveText([
    "Catalog options",
    "Duplicate",
    "Managed by team",
  ]);
  await expect(page.getByRole("menuitem", { name: "Edit" })).toHaveCount(0);
  await expect(
    page.getByRole("menuitem", { name: "Remove from My Agents" }),
  ).toHaveCount(0);
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
    "Fizz is not in My Agents. Choose it from Agent Catalog first.",
  );
});

test("built-in removal failures show up from My Agents", async ({ page }) => {
  await gotoApp(page);

  await page.getByTestId("open-agents-view").click();
  await openPersonaCatalog(page);
  await selectCatalogPersona(page, "builtin:fizz");
  await useCatalogPersona(page, "builtin:fizz");

  await invokeTauri(page, "create_team", {
    input: {
      name: "Fizzes",
      personaIds: ["builtin:fizz"],
    },
  });

  await page.keyboard.press("Escape");
  await page.getByLabel("Open actions for Fizz").click();
  await page.getByRole("menuitem", { name: "Remove from My Agents" }).click();

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
