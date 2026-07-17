import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

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
  for (const personaName of ["Fizz", "Honey", "Bumble"]) {
    await expect(page.getByTestId("persona-catalog-dialog")).toContainText(
      personaName,
    );
  }
  for (const retiredPersonaName of [
    "Product Strategist",
    "Implementation Partner",
    "QA Reviewer",
    "Work Coordinator",
    "Support Guide",
    "Experiment Designer",
  ]) {
    await expect(page.getByTestId("persona-catalog-dialog")).not.toContainText(
      retiredPersonaName,
    );
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
    "Delete",
  );
  await expect.poll(() => getCatalogOrder(page)).toEqual(initialCatalogOrder);
});

test("built-in persona edits persist", async ({ page }) => {
  await installMockBridge(page, {
    activePersonaIds: ["builtin:fizz"],
    globalAgentConfig: {
      env_vars: { ANTHROPIC_API_KEY: "sk-ant-test" },
      provider: "anthropic",
      model: "claude-opus-4-5",
    },
  });
  await gotoApp(page);
  await page.getByTestId("open-agents-view").click();

  await page.getByLabel("Open actions for Fizz").click();
  await page.getByRole("menuitem", { name: "Edit" }).click();

  const dialog = page.getByTestId("persona-dialog");
  await dialog.getByLabel("Agent name").fill("My Fizz");
  await dialog.getByLabel("Agent instruction").fill("User-edited instructions");
  await dialog.getByRole("button", { name: "Save changes" }).click();

  await expect(dialog).toHaveCount(0);
  await expect(page.getByTestId("agents-library-personas")).toContainText(
    "My Fizz",
  );
  const personas = await invokeTauri<
    Array<{ id: string; display_name: string; system_prompt: string }>
  >(page, "list_personas");
  expect(
    personas.find((persona) => persona.id === "builtin:fizz"),
  ).toMatchObject({
    display_name: "My Fizz",
    system_prompt: "User-edited instructions",
  });
});

test("agent avatar emoji picker scrolls inside its popover", async ({
  page,
}) => {
  await gotoApp(page);
  await page.getByTestId("open-agents-view").click();
  await page.getByTestId("new-agent-card").click();
  await page.getByRole("menuitem", { name: "New agent" }).click();

  await expect(page.getByTestId("persona-dialog")).toBeVisible();
  await page.getByLabel("Add avatar").click();
  await page.getByRole("tab", { name: "Emoji" }).click();
  await expect(page.locator("em-emoji-picker")).toBeVisible();

  await page.waitForFunction(() => {
    const picker = document.querySelector("em-emoji-picker");
    const scroll = picker?.shadowRoot?.querySelector(".scroll");
    return (
      scroll instanceof HTMLElement && scroll.scrollHeight > scroll.clientHeight
    );
  });

  const before = await page.locator("em-emoji-picker").evaluate((picker) => {
    const scroll = picker.shadowRoot?.querySelector(".scroll");
    return scroll instanceof HTMLElement ? scroll.scrollTop : -1;
  });

  const box = await page.locator("em-emoji-picker").boundingBox();
  if (!box) {
    throw new Error("Could not measure emoji picker bounds.");
  }
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.wheel(0, 500);

  await expect
    .poll(async () =>
      page.locator("em-emoji-picker").evaluate((picker) => {
        const scroll = picker.shadowRoot?.querySelector(".scroll");
        return scroll instanceof HTMLElement ? scroll.scrollTop : -1;
      }),
    )
    .toBeGreaterThan(before);
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
    "Built-in agent",
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

type AgentShareCommand = { command: string; payload: unknown };

async function openSafetyShareDialog(
  page: import("@playwright/test").Page,
  options: Parameters<typeof installMockBridge>[1] = {},
) {
  await installMockBridge(page, {
    personas: [
      {
        id: "custom:safety-auditor",
        displayName: "Safety Auditor",
        systemPrompt: "You audit safety boundaries.",
      },
    ],
    searchProfiles: [
      {
        pubkey: TEST_IDENTITIES.charlie.pubkey,
        displayName: "Charlie",
      },
    ],
    uploadDescriptors: [
      {
        url: `https://mock.relay/media/${"d".repeat(64)}.png`,
        sha256: "d".repeat(64),
        size: 1234,
        type: "image/png",
        uploaded: Math.floor(Date.now() / 1000),
        filename: "safety-auditor.agent.png",
      },
    ],
    ...options,
  });
  await gotoApp(page);
  await page.getByTestId("open-agents-view").click();
  await page.getByLabel("Open actions for Safety Auditor").click();
  await page.getByRole("menuitem", { name: "Share" }).click();
  await expect(page.getByTestId("persona-share-dialog")).toBeVisible();
}

async function selectCharlieRecipient(page: import("@playwright/test").Page) {
  const search = page.getByTestId("persona-share-recipient-search");
  await expect(search).toBeEnabled({ timeout: 5_000 });
  await search.fill("charlie");
  await page
    .getByTestId(
      `persona-share-recipient-option-${TEST_IDENTITIES.charlie.pubkey}`,
    )
    .click();
}

async function readAgentShareCommands(
  page: import("@playwright/test").Page,
): Promise<AgentShareCommand[]> {
  return page.evaluate(
    () =>
      (
        window as Window & {
          __BUZZ_E2E_COMMAND_LOG__?: AgentShareCommand[];
        }
      ).__BUZZ_E2E_COMMAND_LOG__ ?? [],
  );
}

test("custom personas share with people and keep export separate", async ({
  page,
}) => {
  const sharedAgentUrl = `https://mock.relay/media/${"b".repeat(64)}.png`;
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"]);
  await installMockBridge(page, {
    personas: [
      {
        id: "custom:analyst",
        displayName: "Animation Auditor",
        systemPrompt: "You audit animations.",
      },
    ],
    searchProfiles: [
      {
        pubkey: TEST_IDENTITIES.charlie.pubkey,
        displayName: "Charlie",
      },
      {
        pubkey: TEST_IDENTITIES.bob.pubkey,
        displayName: "Bob",
      },
      ...Array.from({ length: 12 }, (_, index) => ({
        pubkey: (index + 10).toString(16).padStart(64, "0"),
        displayName: `Person ${index + 1}`,
      })),
    ],
    uploadDescriptors: [
      {
        url: sharedAgentUrl,
        sha256: "b".repeat(64),
        size: 1234,
        type: "image/png",
        uploaded: Math.floor(Date.now() / 1000),
        filename: "analyst.agent.png",
      },
    ],
  });
  await gotoApp(page);

  await page.getByTestId("open-agents-view").click();
  await expect(page.getByTestId("agents-library-personas")).toContainText(
    "Animation Auditor",
  );

  const actionsButton = page.getByLabel("Open actions for Animation Auditor");
  await expect(actionsButton.locator("svg")).toHaveClass(
    /lucide-ellipsis-vertical/,
  );
  await actionsButton.click();
  await expect(page.getByRole("menuitem")).toHaveText([
    "Edit",
    "Duplicate",
    "Share",
    "Delete",
  ]);
  await page.getByRole("menuitem", { name: "Share" }).click();

  const shareDialog = page.getByTestId("persona-share-dialog");
  await expect(shareDialog).toBeVisible();
  const emptyRecipientSearch = shareDialog.getByTestId(
    "persona-share-recipient-search",
  );
  const [searchIconColor, searchPlaceholderColor, searchInputColor] =
    await Promise.all([
      shareDialog
        .locator("svg.lucide-search")
        .evaluate((element) => getComputedStyle(element).color),
      emptyRecipientSearch.evaluate(
        (element) => getComputedStyle(element, "::placeholder").color,
      ),
      emptyRecipientSearch.evaluate(
        (element) => getComputedStyle(element).color,
      ),
    ]);
  expect(searchIconColor).toBe(searchPlaceholderColor);
  expect(searchPlaceholderColor).not.toBe(searchInputColor);
  const shareCloseButton = shareDialog.getByRole("button", { name: "Close" });
  expect(
    await shareCloseButton.evaluate(
      (button) => getComputedStyle(button).boxShadow,
    ),
  ).not.toContain("1px");
  await expect(
    page.getByRole("heading", { name: "Share Animation Auditor" }),
  ).toBeVisible();
  await expect(shareDialog.getByText("Added by You")).toHaveCount(0);
  const sendDescription = shareDialog.getByTestId(
    "persona-share-send-description",
  );
  await expect(sendDescription).toHaveText(
    "They’ll receive a copy they can add and use. Changes you make later won’t sync.",
  );
  await expect(sendDescription).toHaveClass(
    /text-xs.*text-secondary-foreground\/75/,
  );
  const sendDescriptionMetrics = await sendDescription.evaluate((element) => ({
    height: element.getBoundingClientRect().height,
    lineHeight: Number.parseFloat(getComputedStyle(element).lineHeight),
  }));
  expect(sendDescriptionMetrics.height).toBeLessThanOrEqual(
    sendDescriptionMetrics.lineHeight + 1,
  );
  await expect(
    shareDialog.getByRole("heading", { name: "Who has access" }),
  ).toHaveCount(0);
  await expect(shareDialog.getByText("Owner", { exact: true })).toHaveCount(0);
  await expect(shareDialog.getByText("(You)", { exact: true })).toHaveCount(0);
  const copyLinkFooter = page.getByTestId("persona-share-copy-link-footer");
  await expect(
    copyLinkFooter.getByRole("heading", { name: "Share with a link" }),
  ).toBeVisible();
  await expect(
    copyLinkFooter.getByText("Anyone with the link can add and use a copy."),
  ).toHaveClass(/text-xs.*text-secondary-foreground\/75/);
  await expect(page.getByTestId("persona-share-send")).toHaveCount(0);
  const copyLinkButton = page.getByTestId("persona-share-copy-link");
  const linkRow = page.getByTestId("persona-share-link-row");
  const linkIcon = page.getByTestId("persona-share-link-icon");
  const linkCopy = page.getByTestId("persona-share-link-copy");
  const linkDivider = page.getByTestId("persona-share-link-divider");
  const staticLinkAccess = page.getByTestId("persona-share-link-access");
  await waitForAnimations(page);
  const [
    linkRowBox,
    initialCopyLinkButtonBox,
    linkIconBox,
    linkCopyBox,
    linkDividerBox,
    staticLinkAccessBox,
  ] = await Promise.all([
    linkRow.boundingBox(),
    copyLinkButton.boundingBox(),
    linkIcon.boundingBox(),
    linkCopy.boundingBox(),
    linkDivider.boundingBox(),
    staticLinkAccess.boundingBox(),
  ]);
  const sendDescriptionBox = await sendDescription.boundingBox();
  expect((linkRowBox?.y ?? 0) - (sendDescriptionBox?.y ?? 0)).toBeGreaterThan(
    (sendDescriptionBox?.height ?? 0) + 30,
  );
  expect(initialCopyLinkButtonBox?.y ?? 0).toBeGreaterThanOrEqual(
    (linkRowBox?.y ?? 0) + (linkRowBox?.height ?? 0) + 23,
  );
  expect(
    Math.abs(
      (linkCopyBox?.y ?? 0) +
        (linkCopyBox?.height ?? 0) / 2 -
        ((linkIconBox?.y ?? 0) + (linkIconBox?.height ?? 0) / 2),
    ),
  ).toBeLessThanOrEqual(1);
  expect(linkDividerBox?.y ?? 0).toBeGreaterThan(
    (linkRowBox?.y ?? 0) + (linkRowBox?.height ?? 0),
  );
  expect(linkDividerBox?.y ?? 0).toBeLessThan(initialCopyLinkButtonBox?.y ?? 0);
  expect(
    Math.abs((linkDividerBox?.width ?? 0) - (linkRowBox?.width ?? 0)),
  ).toBeLessThanOrEqual(1);
  await expect(linkDivider).toHaveClass(/my-4.*bg-input\/40/);
  expect(
    Math.abs(
      (linkCopyBox?.y ?? 0) +
        (linkCopyBox?.height ?? 0) / 2 -
        ((staticLinkAccessBox?.y ?? 0) +
          (staticLinkAccessBox?.height ?? 0) / 2),
    ),
  ).toBeLessThanOrEqual(1);
  const shareMainCardForLinkSpacing = page.getByTestId(
    "persona-share-main-card",
  );
  const shareMainCardForLinkSpacingBox =
    await shareMainCardForLinkSpacing.boundingBox();
  const gapAboveCopyLink =
    (initialCopyLinkButtonBox?.y ?? 0) -
    ((linkDividerBox?.y ?? 0) + (linkDividerBox?.height ?? 0));
  const gapBelowCopyLink =
    (shareMainCardForLinkSpacingBox?.y ?? 0) +
    (shareMainCardForLinkSpacingBox?.height ?? 0) -
    ((initialCopyLinkButtonBox?.y ?? 0) +
      (initialCopyLinkButtonBox?.height ?? 0));
  expect(Math.abs(gapAboveCopyLink - gapBelowCopyLink)).toBeLessThanOrEqual(1);
  await expect(copyLinkButton).toHaveClass(
    /border.*bg-background.*border-border/,
  );
  const copyLinkHasVisibleShadow = () =>
    copyLinkButton.evaluate((element) => {
      const boxShadow = getComputedStyle(element).boxShadow;
      if (boxShadow === "none") return false;
      return [...boxShadow.matchAll(/rgba?\(([^)]+)\)/g)].some((match) => {
        if (match[0].startsWith("rgb(")) return true;
        const channels = match[1]?.split(/[\s,/]+/).filter(Boolean) ?? [];
        return Number(channels.at(-1)) > 0;
      });
    });
  await expect.poll(copyLinkHasVisibleShadow).toBe(false);
  await copyLinkButton.hover();
  await expect.poll(copyLinkHasVisibleShadow).toBe(false);
  await expect(page.getByTestId("persona-share-link-access")).toHaveText(
    "Agent only",
  );
  await expect(page.getByTestId("persona-share-recipient-access")).toHaveCount(
    0,
  );
  await expect(
    shareDialog.getByLabel("What to include in the link"),
  ).toHaveCount(0);
  await expect(
    shareDialog.getByLabel("What to include", { exact: true }),
  ).toHaveCount(0);
  await expect(shareDialog.getByText("Memories")).toHaveCount(0);
  await expect(shareDialog.getByText("File format")).toHaveCount(0);
  await expect(page.getByText("Show in my catalog")).toHaveCount(0);
  const shareMainCard = page.getByTestId("persona-share-main-card");
  const exportAgentRow = page.getByTestId("persona-share-export");
  await expect(exportAgentRow).toHaveText("Export agent");
  await expect(shareMainCard.getByTestId("persona-share-export")).toHaveCount(
    0,
  );
  await waitForAnimations(page);
  const shareMainCardBox = await shareMainCard.boundingBox();
  const exportAgentRowBox = await exportAgentRow.boundingBox();
  const shareCardGap =
    (exportAgentRowBox?.y ?? 0) -
    ((shareMainCardBox?.y ?? 0) + (shareMainCardBox?.height ?? 0));
  expect(shareCardGap).toBeGreaterThanOrEqual(12);
  expect(shareCardGap).toBeLessThan(16);
  const [shareMainCardStyles, exportAgentRowStyles] = await Promise.all([
    shareMainCard.evaluate((element) => ({
      borderRadius: getComputedStyle(element).borderRadius,
      boxShadow: getComputedStyle(element).boxShadow,
    })),
    exportAgentRow.evaluate((element) => ({
      borderRadius: getComputedStyle(element).borderRadius,
      boxShadow: getComputedStyle(element).boxShadow,
    })),
  ]);
  expect(exportAgentRowStyles.borderRadius).toBe(
    shareMainCardStyles.borderRadius,
  );
  const shareMainCardShadow = shareMainCardStyles.boxShadow;
  const exportAgentRowShadow = exportAgentRowStyles.boxShadow;
  expect(exportAgentRowShadow).toBe(shareMainCardShadow);
  expect(exportAgentRowShadow).not.toBe("none");
  await expect(exportAgentRow).toHaveCSS("position", "relative");
  await expect(page.getByTestId("agent-snapshot-export-dialog")).toHaveCount(0);

  await exportAgentRow.click();
  await expect(shareDialog).toHaveCount(0);
  const exportDialog = page.getByTestId("agent-snapshot-export-dialog");
  await expect(exportDialog).toBeVisible();
  const exportDialogBox = await exportDialog.boundingBox();
  expect(exportDialogBox?.width).toBeLessThanOrEqual(448);
  await expect(
    exportDialog.getByRole("heading", {
      name: "Export Animation Auditor",
      exact: true,
    }),
  ).toBeVisible();
  await expect(exportDialog.getByText("portable snapshot")).toHaveCount(0);
  await expect(
    exportDialog.getByRole("button", { name: "Send in Buzz" }),
  ).toHaveCount(0);
  await expect(exportDialog.getByLabel("Memories")).toHaveCount(0);
  await expect(
    exportDialog.getByTestId("agent-snapshot-memory-value"),
  ).toHaveText("Agent only");
  await expect(exportDialog.getByText("Start this agent")).toHaveCount(0);
  await expect(
    exportDialog.getByTestId("agent-snapshot-memory-value").locator("svg"),
  ).toHaveCount(0);
  const formatTrigger = exportDialog.getByLabel("File format");
  await expect(formatTrigger).toHaveText("PNG");
  expect((await formatTrigger.boundingBox())?.width).toBeLessThan(80);
  expect(await formatTrigger.evaluate((element) => element.tagName)).toBe(
    "BUTTON",
  );
  await formatTrigger.click();
  await expect(page.getByRole("menuitemradio", { name: "JSON" })).toBeVisible();
  await page.getByRole("menuitemradio", { name: "PNG" }).click();
  const exportFooter = exportDialog.getByTestId("agent-snapshot-export-footer");
  const cancelButtonBox = await exportFooter
    .getByRole("button", { name: "Cancel" })
    .boundingBox();
  const exportButtonBox = await exportFooter
    .getByRole("button", { name: "Export" })
    .boundingBox();
  expect(cancelButtonBox?.x).toBeLessThan(exportButtonBox?.x ?? 0);
  expect(
    (exportButtonBox?.x ?? 0) -
      ((cancelButtonBox?.x ?? 0) + (cancelButtonBox?.width ?? 0)),
  ).toBeLessThanOrEqual(12);
  await exportDialog.getByRole("button", { name: "Cancel" }).click();

  await actionsButton.click();
  await page.getByRole("menuitem", { name: "Share" }).click();
  await expect(shareDialog).toBeVisible();
  await page.getByTestId("persona-share-copy-link").click();
  await expect
    .poll(() => page.evaluate(() => navigator.clipboard.readText()))
    .toBe(sharedAgentUrl);

  const copiedAgent = await page.evaluate(() => {
    const commands =
      (
        window as Window & {
          __BUZZ_E2E_COMMAND_LOG__?: Array<{
            command: string;
            payload: { html?: string; text?: string };
          }>;
        }
      ).__BUZZ_E2E_COMMAND_LOG__ ?? [];
    return commands.findLast(
      (entry) => entry.command === "copy_text_to_clipboard",
    )?.payload;
  });
  expect(copiedAgent?.text).toBe(sharedAgentUrl);
  expect(copiedAgent?.html).toContain("data-buzz-agent-snapshot");

  await page.keyboard.press("Escape");
  await expect(shareDialog).toHaveCount(0);
  await page.getByTestId("channel-general").click();
  await page.evaluate(async ({ html, text }) => {
    await navigator.clipboard.write([
      new ClipboardItem({
        "text/html": new Blob([html ?? ""], { type: "text/html" }),
        "text/plain": new Blob([text ?? ""], { type: "text/plain" }),
      }),
    ]);
  }, copiedAgent ?? {});
  await page
    .getByTestId("message-composer")
    .locator("[contenteditable='true']")
    .click();
  await page.keyboard.press("ControlOrMeta+V");
  const composerAgentCard = page.getByTestId("composer-agent-snapshot-card");
  await expect(composerAgentCard).toBeVisible();
  await expect(composerAgentCard).toContainText("Animation Auditor");
  const composerRemoveButton = page.getByTestId(
    "composer-agent-snapshot-remove",
  );
  await expect(composerRemoveButton).toHaveCSS("border-top-width", "0px");
  await composerRemoveButton.focus();
  await page.keyboard.press("Tab");
  await page.keyboard.press("Shift+Tab");
  await expect(composerRemoveButton).toBeFocused();
  expect(
    await composerRemoveButton.evaluate(
      (button) => getComputedStyle(button).boxShadow,
    ),
  ).not.toContain("1px");
  await expect(composerRemoveButton).not.toHaveCSS(
    "background-color",
    "rgba(0, 0, 0, 0)",
  );
  await expect(page.getByTestId("send-message")).toBeEnabled();
  await page.getByTestId("send-message").click();

  const pastedAgentCard = page.getByTestId("agent-snapshot-card").last();
  await expect(pastedAgentCard).toBeVisible();
  await expect(pastedAgentCard).toContainText("Animation Auditor");

  await page.getByTestId("open-agents-view").click();
  await actionsButton.click();
  await page.getByRole("menuitem", { name: "Share" }).click();
  await expect(shareDialog).toBeVisible();

  const recipientSearch = page.getByTestId("persona-share-recipient-search");
  const recipientField = page.getByTestId("persona-share-recipient-field");
  await waitForAnimations(page);
  const emptyRecipientFieldWidth =
    (await recipientField.boundingBox())?.width ?? 0;
  await recipientSearch.click();
  const recipientPopover = page.getByTestId("persona-share-recipient-popover");
  await expect(recipientPopover).toBeVisible();
  await page.evaluate(() => {
    const content = document.querySelector(
      '[data-testid="persona-share-recipient-popover"]',
    );
    if (!content) throw new Error("Recipient popover is not mounted");

    const stateChanges: string[] = [];
    const observer = new MutationObserver(() => {
      stateChanges.push(content.getAttribute("data-state") ?? "unmounted");
    });
    observer.observe(content, {
      attributeFilter: ["data-state"],
      attributes: true,
    });
    (
      window as Window & {
        __BUZZ_RECIPIENT_POPOVER_OBSERVER__?: MutationObserver;
        __BUZZ_RECIPIENT_POPOVER_STATE_CHANGES__?: string[];
      }
    ).__BUZZ_RECIPIENT_POPOVER_OBSERVER__ = observer;
    (
      window as Window & {
        __BUZZ_RECIPIENT_POPOVER_STATE_CHANGES__?: string[];
      }
    ).__BUZZ_RECIPIENT_POPOVER_STATE_CHANGES__ = stateChanges;
  });
  await recipientSearch.click();
  await waitForAnimations(page);
  const recipientPopoverStateChanges = await page.evaluate(() => {
    const trackedWindow = window as Window & {
      __BUZZ_RECIPIENT_POPOVER_OBSERVER__?: MutationObserver;
      __BUZZ_RECIPIENT_POPOVER_STATE_CHANGES__?: string[];
    };
    trackedWindow.__BUZZ_RECIPIENT_POPOVER_OBSERVER__?.disconnect();
    return trackedWindow.__BUZZ_RECIPIENT_POPOVER_STATE_CHANGES__ ?? [];
  });
  expect(recipientPopoverStateChanges).not.toContain("closed");
  const recipientList = page.getByTestId("persona-share-recipient-results");
  await expect(recipientList).toBeVisible();
  await expect
    .poll(() =>
      recipientList.evaluate(
        (element) => element.scrollHeight - element.clientHeight,
      ),
    )
    .toBeGreaterThan(0);
  const recipientListBox = await recipientList.boundingBox();
  expect(recipientListBox).not.toBeNull();
  if (!recipientListBox) return;
  await page.mouse.move(
    recipientListBox.x + recipientListBox.width / 2,
    recipientListBox.y + recipientListBox.height / 2,
  );
  await page.mouse.wheel(0, 300);
  await expect
    .poll(() => recipientList.evaluate((element) => element.scrollTop))
    .toBeGreaterThan(0);
  await recipientSearch.fill("charlie");
  await page
    .getByTestId(
      `persona-share-recipient-option-${TEST_IDENTITIES.charlie.pubkey}`,
    )
    .click();
  const charlieChip = page.getByTestId(
    `persona-share-recipient-chip-${TEST_IDENTITIES.charlie.pubkey}`,
  );
  await expect(charlieChip).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Verify charlie public key" }),
  ).toHaveCount(0);
  await expect(recipientSearch).toHaveValue("");
  await expect(recipientSearch).toHaveAttribute("placeholder", "");
  await expect(
    page
      .getByTestId("persona-share-recipient-field")
      .locator("svg.lucide-search"),
  ).toHaveCount(0);
  await expect(
    page
      .getByTestId("persona-share-recipient-field")
      .getByTestId("persona-share-recipient-access"),
  ).toHaveText("Agent only");
  const staticRecipientAccess = page.getByTestId(
    "persona-share-recipient-access",
  );
  const [
    staticRecipientAccessBox,
    recipientAccessPaddingRight,
    recipientFieldBox,
  ] = await Promise.all([
    staticRecipientAccess.boundingBox(),
    staticRecipientAccess.evaluate((element) =>
      Number.parseFloat(getComputedStyle(element).paddingRight),
    ),
    recipientField.boundingBox(),
  ]);
  const staticRecipientTextInset =
    (recipientFieldBox?.x ?? 0) +
    (recipientFieldBox?.width ?? 0) -
    ((staticRecipientAccessBox?.x ?? 0) +
      (staticRecipientAccessBox?.width ?? 0) -
      recipientAccessPaddingRight);
  expect(staticRecipientTextInset).toBeGreaterThanOrEqual(8);
  expect(staticRecipientTextInset).toBeLessThanOrEqual(10);
  await expect(page.getByTestId("persona-share-send")).toBeVisible();

  await recipientSearch.fill("bob");
  await page
    .getByTestId(`persona-share-recipient-option-${TEST_IDENTITIES.bob.pubkey}`)
    .click();
  const bobChip = page.getByTestId(
    `persona-share-recipient-chip-${TEST_IDENTITIES.bob.pubkey}`,
  );
  await expect(bobChip).toBeVisible();
  await expect(bobChip).not.toHaveClass(/buzz-poof-trigger/);
  await waitForAnimations(page);
  const bobChipBox = await bobChip.boundingBox();
  expect(bobChipBox).not.toBeNull();
  if (!bobChipBox) return;
  const removePointer = {
    x: bobChipBox.x + bobChipBox.width - 3,
    y: bobChipBox.y + bobChipBox.height / 2,
  };
  await page.mouse.move(removePointer.x, removePointer.y);
  await page.mouse.down();
  await expect(page.locator(".buzz-poof-burst")).toHaveCount(0);
  await page.mouse.up();
  await expect(bobChip).toHaveCount(0);

  await waitForAnimations(page);
  const selectedRecipientFieldWidth =
    (await recipientField.boundingBox())?.width ?? 0;
  const charlieChipBox = await charlieChip.boundingBox();
  expect(charlieChipBox).not.toBeNull();
  if (!charlieChipBox) return;
  await page.mouse.move(
    charlieChipBox.x + charlieChipBox.width / 2,
    charlieChipBox.y + charlieChipBox.height / 2,
  );
  await page.mouse.down();
  const lastChipRemovalSamples = await page.evaluate(async () => {
    const samples: Array<{ elapsed: number; fieldWidth: number }> = [];
    const start = performance.now();

    await new Promise<void>((resolve) => {
      const sample = (now: number) => {
        const field = document.querySelector<HTMLElement>(
          '[data-testid="persona-share-recipient-field"]',
        );
        samples.push({
          elapsed: now - start,
          fieldWidth: field?.getBoundingClientRect().width ?? 0,
        });
        if (now - start >= 240) {
          resolve();
          return;
        }
        requestAnimationFrame(sample);
      };
      requestAnimationFrame(sample);
    });

    return samples;
  });
  await page.mouse.up();
  await expect(charlieChip).toHaveCount(0);
  const firstExpandedSample = lastChipRemovalSamples.find(
    ({ fieldWidth }) => fieldWidth > selectedRecipientFieldWidth + 1,
  );
  expect(firstExpandedSample?.elapsed).toBeLessThan(100);
  expect(
    Math.abs(
      (lastChipRemovalSamples.at(-1)?.fieldWidth ?? 0) -
        emptyRecipientFieldWidth,
    ),
  ).toBeLessThanOrEqual(1);

  await recipientSearch.fill("bob");
  await page
    .getByTestId(`persona-share-recipient-option-${TEST_IDENTITIES.bob.pubkey}`)
    .click();
  await page.getByTestId("persona-share-send").click();
  await expect(
    page.getByText("Sent a copy of Animation Auditor"),
  ).toBeVisible();

  const sentAgentMessages = await page.evaluate(() =>
    (
      (
        window as Window & {
          __BUZZ_E2E_COMMAND_LOG__?: Array<{
            command: string;
            payload: { content?: string };
          }>;
        }
      ).__BUZZ_E2E_COMMAND_LOG__ ?? []
    ).filter((entry) => entry.command === "send_channel_message"),
  );
  expect(sentAgentMessages.at(-1)?.payload.content).toBe(
    `\n[Animation Auditor](${sharedAgentUrl})`,
  );
  await expect(shareDialog).toHaveCount(0);
});

test("share access controls include the selected memories", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "no-preference" });
  const linkedAgentPubkey = TEST_IDENTITIES.alice.pubkey;
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"]);
  await installMockBridge(page, {
    personas: [
      {
        id: "custom:animation-auditor",
        displayName: "Animation Auditor",
        systemPrompt: "You audit animations.",
      },
    ],
    managedAgents: [
      {
        pubkey: linkedAgentPubkey,
        name: "Animation Auditor",
        personaId: "custom:animation-auditor",
        status: "running",
      },
    ],
    searchProfiles: [
      {
        pubkey: TEST_IDENTITIES.charlie.pubkey,
        displayName: "Charlie",
      },
    ],
    uploadDescriptors: [
      {
        url: `https://mock.relay/media/${"c".repeat(64)}.png`,
        sha256: "c".repeat(64),
        size: 1234,
        type: "image/png",
        uploaded: Math.floor(Date.now() / 1000),
        filename: "animation-auditor.agent.png",
      },
    ],
  });
  await gotoApp(page);

  await page.getByTestId("open-agents-view").click();
  await page.getByLabel("Open actions for Animation Auditor").click();
  await page.getByRole("menuitem", { name: "Share" }).click();

  const shareDialog = page.getByTestId("persona-share-dialog");
  const shareMainCard = shareDialog.getByTestId("persona-share-main-card");
  const initialShareCardHeight = await shareMainCard.evaluate(
    (element) => element.getBoundingClientRect().height,
  );
  const linkAccess = shareDialog.getByLabel("What to include in the link");
  const recipientField = page.getByTestId("persona-share-recipient-field");
  const emptyRecipientFieldBox = await recipientField.boundingBox();
  await expect(shareDialog.getByTestId("persona-share-send")).toHaveCount(0);
  await expect(linkAccess).toHaveText("Agent only");
  expect((await linkAccess.boundingBox())?.width).toBeLessThan(120);
  expect(await linkAccess.evaluate((element) => element.tagName)).toBe(
    "BUTTON",
  );
  await expect(linkAccess).toHaveCSS("text-decoration-line", "none");
  await expect(linkAccess).toHaveCSS("padding-left", "8px");
  await expect(linkAccess).toHaveCSS("padding-right", "8px");
  const copyLinkButton = shareDialog.getByTestId("persona-share-copy-link");
  const [linkAccessBox, copyLinkButtonBox] = await Promise.all([
    linkAccess.boundingBox(),
    copyLinkButton.boundingBox(),
  ]);
  expect(copyLinkButtonBox?.y ?? 0).toBeGreaterThanOrEqual(
    (linkAccessBox?.y ?? 0) + (linkAccessBox?.height ?? 0) + 8,
  );
  await expect(
    shareDialog.getByLabel("What to include", { exact: true }),
  ).toHaveCount(0);
  await expect(
    shareDialog.getByTestId("persona-share-memory-warning"),
  ).toHaveCount(0);

  await linkAccess.click();
  await expect(page.getByRole("menuitemradio")).toHaveText([
    "Agent only",
    "Agent + core memory",
    "Agent + all memories",
  ]);
  await page
    .getByRole("menuitemradio", { name: "Agent + core memory" })
    .click();
  await expect(linkAccess).toHaveText("Agent + core memory");
  await waitForAnimations(page);
  const expandedShareCardHeight = await shareMainCard.evaluate(
    (element) => element.getBoundingClientRect().height,
  );
  const inlineMemoryWarning = shareDialog.getByTestId(
    "persona-share-memory-warning",
  );
  await expect(inlineMemoryWarning).toBeVisible();
  await expect(inlineMemoryWarning).toContainText(
    "Memory is stored as plaintext in the snapshot.",
  );
  await expect(inlineMemoryWarning).toContainText(
    "Only share it with people you trust.",
  );
  expect(expandedShareCardHeight).toBeGreaterThan(initialShareCardHeight);
  await page.getByTestId("persona-share-copy-link").click();
  const memoryConfirmation = page.getByTestId(
    "persona-share-memory-confirmation",
  );
  await expect(memoryConfirmation).toBeVisible();
  await expect(
    memoryConfirmation.getByRole("heading", { name: "Share memories?" }),
  ).toBeVisible();
  await expect(memoryConfirmation).toContainText("plaintext core memory");
  await expect(memoryConfirmation).toContainText(
    "Anyone with the link can view it.",
  );
  await expect(memoryConfirmation).toContainText(
    "Only share with people you trust.",
  );
  const encodeLevelsBeforeLinkConfirmation = await page.evaluate(() =>
    (
      (
        window as Window & {
          __BUZZ_E2E_COMMAND_LOG__?: Array<{
            command: string;
            payload: { memoryLevel?: string };
          }>;
        }
      ).__BUZZ_E2E_COMMAND_LOG__ ?? []
    )
      .filter((entry) => entry.command === "encode_agent_snapshot_for_send")
      .map((entry) => entry.payload.memoryLevel),
  );
  expect(encodeLevelsBeforeLinkConfirmation).toEqual([]);
  await memoryConfirmation.getByTestId("persona-share-memory-confirm").click();
  await expect(page.getByTestId("persona-share-copy-link")).toContainText(
    "Copied",
  );
  await linkAccess.click();
  await page
    .getByRole("menuitemradio", { name: "Agent only", exact: true })
    .click();
  await expect(linkAccess).toHaveText("Agent only");
  await expect(inlineMemoryWarning).toHaveCount(0);

  const recipientSearch = page.getByTestId("persona-share-recipient-search");
  await recipientSearch.fill("charlie");
  await page
    .getByTestId(
      `persona-share-recipient-option-${TEST_IDENTITIES.charlie.pubkey}`,
    )
    .click();
  const recipientInputRegion = recipientField.getByTestId(
    "persona-share-recipient-input-region",
  );
  const recipientAccess = recipientField.getByLabel("What to include", {
    exact: true,
  });
  await expect(recipientAccess).toHaveText("Agent only");
  expect((await recipientAccess.boundingBox())?.width).toBeLessThan(140);
  await expect(recipientField).toHaveCSS("column-gap", "12px");
  await expect(recipientInputRegion).toHaveCSS("flex-wrap", "wrap");
  const sendButton = shareDialog.getByTestId("persona-share-send");
  await expect(sendButton).toBeVisible();
  await waitForAnimations(page);
  const resizedRecipientFieldBox = await recipientField.boundingBox();
  expect(resizedRecipientFieldBox?.width).toBeLessThan(
    emptyRecipientFieldBox?.width ?? 0,
  );
  await expect
    .poll(async () => {
      const [sendButtonBox, currentCopyLinkButtonBox] = await Promise.all([
        sendButton.boundingBox(),
        copyLinkButton.boundingBox(),
      ]);
      return Math.abs(
        (sendButtonBox?.x ?? 0) +
          (sendButtonBox?.width ?? 0) -
          ((currentCopyLinkButtonBox?.x ?? 0) +
            (currentCopyLinkButtonBox?.width ?? 0)),
      );
    })
    .toBeLessThanOrEqual(1);
  const recipientInputRegionBox = await recipientInputRegion.boundingBox();
  const recipientAccessBox = await recipientAccess.boundingBox();
  expect(
    (recipientAccessBox?.x ?? 0) -
      ((recipientInputRegionBox?.x ?? 0) +
        (recipientInputRegionBox?.width ?? 0)),
  ).toBeGreaterThanOrEqual(12);
  const recipientAccessRightEdge =
    (recipientAccessBox?.x ?? 0) + (recipientAccessBox?.width ?? 0);
  expect(
    Math.abs(
      (resizedRecipientFieldBox?.x ?? 0) +
        (resizedRecipientFieldBox?.width ?? 0) -
        8 -
        recipientAccessRightEdge,
    ),
  ).toBeLessThanOrEqual(8);
  await recipientAccess.click();
  await page
    .getByRole("menuitemradio", { name: "Agent + all memories" })
    .click();
  await expect(recipientAccess).toHaveText("Agent + all memories");
  await expect(inlineMemoryWarning).toBeVisible();
  await waitForAnimations(page);
  await expect
    .poll(async () => {
      const expandedRecipientAccessBox = await recipientAccess.boundingBox();
      return Math.abs(
        (expandedRecipientAccessBox?.x ?? 0) +
          (expandedRecipientAccessBox?.width ?? 0) -
          recipientAccessRightEdge,
      );
    })
    .toBeLessThanOrEqual(1);
  expect(
    await recipientAccess
      .locator("span")
      .evaluate((element) => element.scrollWidth <= element.clientWidth),
  ).toBe(true);
  await page.getByTestId("persona-share-send").click();
  await expect(memoryConfirmation).toBeVisible();
  await expect(memoryConfirmation).toContainText("plaintext all memories");
  await expect(memoryConfirmation).toContainText(
    "Charlie—and anyone with the file link—can view it.",
  );
  const encodeLevelsBeforeSendConfirmation = await page.evaluate(() =>
    (
      (
        window as Window & {
          __BUZZ_E2E_COMMAND_LOG__?: Array<{
            command: string;
            payload: { memoryLevel?: string };
          }>;
        }
      ).__BUZZ_E2E_COMMAND_LOG__ ?? []
    )
      .filter((entry) => entry.command === "encode_agent_snapshot_for_send")
      .map((entry) => entry.payload.memoryLevel),
  );
  expect(encodeLevelsBeforeSendConfirmation).not.toContain("none");
  expect(encodeLevelsBeforeSendConfirmation).toContain("core");
  expect(encodeLevelsBeforeSendConfirmation).not.toContain("everything");
  await memoryConfirmation.getByTestId("persona-share-memory-confirm").click();
  await expect(
    page.getByText("Sent a copy of Animation Auditor"),
  ).toBeVisible();

  const encodePayloads = await page.evaluate(() =>
    (
      (
        window as Window & {
          __BUZZ_E2E_COMMAND_LOG__?: Array<{
            command: string;
            payload: unknown;
          }>;
        }
      ).__BUZZ_E2E_COMMAND_LOG__ ?? []
    )
      .filter((entry) => entry.command === "encode_agent_snapshot_for_send")
      .map((entry) => entry.payload),
  );
  expect(
    encodePayloads.filter(
      (payload) => (payload as { memoryLevel?: string }).memoryLevel === "none",
    ),
  ).toHaveLength(0);
  expect(encodePayloads).toEqual(
    expect.arrayContaining([
      expect.objectContaining({
        memoryLevel: "core",
        memorySourcePubkey: linkedAgentPubkey,
      }),
      expect.objectContaining({
        memoryLevel: "everything",
        memorySourcePubkey: linkedAgentPubkey,
      }),
    ]),
  );
});

test("people sharing excludes the moderation recipient", async ({ page }) => {
  await openSafetyShareDialog(page, {
    relaySelf: TEST_IDENTITIES.charlie.pubkey,
    relaySelfDelayMs: 800,
  });

  const search = page.getByTestId("persona-share-recipient-search");
  await expect(search).toBeEnabled({ timeout: 5_000 });
  await search.fill("charlie");
  await expect(
    page.getByTestId(
      `persona-share-recipient-option-${TEST_IDENTITIES.charlie.pubkey}`,
    ),
  ).toHaveCount(0);
  await expect(page.getByText("No people found.")).toBeVisible();

  const commands = await readAgentShareCommands(page);
  expect(
    commands.filter((entry) =>
      [
        "open_dm",
        "encode_agent_snapshot_for_send",
        "upload_media_bytes",
        "send_channel_message",
      ].includes(entry.command),
    ),
  ).toEqual([]);
});

test("people sharing blocks a timeout before encoding or upload", async ({
  page,
}) => {
  await openSafetyShareDialog(page);
  await selectCharlieRecipient(page);
  await page.evaluate(() => {
    (
      window as Window & {
        __BUZZ_E2E_ACTIVATE_TIMEOUT__?: (expiresAtMs: number) => void;
      }
    ).__BUZZ_E2E_ACTIVATE_TIMEOUT__?.(Date.now() + 60_000);
  });

  await page.getByTestId("persona-share-send").click();
  await expect(page.getByText("Couldn’t send agent. Try again.")).toBeVisible();

  const commands = await readAgentShareCommands(page);
  expect(
    commands.filter((entry) =>
      [
        "encode_agent_snapshot_for_send",
        "upload_media_bytes",
        "send_channel_message",
      ].includes(entry.command),
    ),
  ).toEqual([]);
});

test("people sharing rechecks destination eligibility after encoding", async ({
  page,
}) => {
  await openSafetyShareDialog(page, { encodeDelayMs: 800 });
  await selectCharlieRecipient(page);
  await page.getByTestId("persona-share-send").click();

  await expect
    .poll(async () => {
      const commands = await readAgentShareCommands(page);
      return commands.filter(
        (entry) => entry.command === "encode_agent_snapshot_for_send",
      ).length;
    })
    .toBe(1);

  await page.evaluate(() => {
    const testWindow = window as Window & {
      __BUZZ_E2E_MUTATE_CHANNEL__?: (options: {
        channelId: string;
        channelType: "forum";
      }) => void;
      __BUZZ_E2E_INVALIDATE_CHANNELS__?: () => Promise<void>;
    };
    testWindow.__BUZZ_E2E_MUTATE_CHANNEL__?.({
      channelId: "d1ec7000-d000-4000-8000-000000000001",
      channelType: "forum",
    });
    return testWindow.__BUZZ_E2E_INVALIDATE_CHANNELS__?.();
  });

  await expect(page.getByText("Couldn’t send agent. Try again.")).toBeVisible({
    timeout: 5_000,
  });
  const commands = await readAgentShareCommands(page);
  expect(
    commands.filter(
      (entry) => entry.command === "encode_agent_snapshot_for_send",
    ),
  ).toHaveLength(1);
  expect(
    commands.filter((entry) => entry.command === "upload_media_bytes"),
  ).toHaveLength(0);
  expect(
    commands.filter((entry) => entry.command === "send_channel_message"),
  ).toHaveLength(0);
});

test("people sharing guards the full action against duplicate sends", async ({
  page,
}) => {
  await openSafetyShareDialog(page, { encodeDelayMs: 500 });
  await selectCharlieRecipient(page);
  await page.getByTestId("persona-share-send").evaluate((button) => {
    if (!(button instanceof HTMLButtonElement)) return;
    button.click();
    button.click();
  });

  await expect(page.getByText("Sent a copy of Safety Auditor")).toBeVisible({
    timeout: 5_000,
  });
  await expect(page.getByText("Couldn’t send agent. Try again.")).toHaveCount(
    0,
  );
  const commands = await readAgentShareCommands(page);
  for (const command of [
    "open_dm",
    "encode_agent_snapshot_for_send",
    "upload_media_bytes",
    "send_channel_message",
  ]) {
    expect(commands.filter((entry) => entry.command === command)).toHaveLength(
      1,
    );
  }
});

test("people sharing stays mounted while a send is pending", async ({
  page,
}) => {
  await openSafetyShareDialog(page, { encodeDelayMs: 800 });
  await selectCharlieRecipient(page);
  await page.getByTestId("persona-share-send").click();

  await page.keyboard.press("Escape");
  await expect(page.getByTestId("persona-share-dialog")).toBeVisible();
  await expect(page.getByText("Sent a copy of Safety Auditor")).toBeVisible({
    timeout: 5_000,
  });
});

test("export from share aligns selections and animates memory details", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await installMockBridge(page, {
    personas: [
      {
        id: "custom:animation-auditor",
        displayName: "Animation Auditor",
        systemPrompt: "You audit animations.",
      },
    ],
    managedAgents: [
      {
        pubkey: TEST_IDENTITIES.alice.pubkey,
        name: "Animation Auditor",
        personaId: "custom:animation-auditor",
        status: "running",
      },
    ],
  });
  await gotoApp(page);

  await page.getByTestId("open-agents-view").click();
  await page.getByLabel("Open actions for Animation Auditor").click();
  await page.getByRole("menuitem", { name: "Share" }).click();
  await page.getByTestId("persona-share-export").click();

  const exportDialog = page.getByTestId("agent-snapshot-export-dialog");
  const memoryTrigger = exportDialog.getByLabel("Memories");
  await expect(memoryTrigger).toHaveText("Agent only");
  expect((await memoryTrigger.boundingBox())?.width).toBeLessThan(112);
  await expect(memoryTrigger).toHaveCSS("text-align", "right");
  await expect(memoryTrigger.locator("svg.lucide-chevron-down")).toBeVisible();

  const initialHeight = await exportDialog.evaluate(
    (element) => element.getBoundingClientRect().height,
  );
  await memoryTrigger.click();
  await page
    .getByRole("menuitemradio", { name: "Agent + core memory" })
    .click();
  const heightSamples = await exportDialog.evaluate(async (element) => {
    const samples: number[] = [];
    const start = performance.now();

    await new Promise<void>((resolve) => {
      const sample = (now: number) => {
        samples.push(element.getBoundingClientRect().height);
        if (now - start >= 280) {
          resolve();
          return;
        }
        requestAnimationFrame(sample);
      };
      requestAnimationFrame(sample);
    });

    return samples;
  });

  await expect(
    exportDialog.getByTestId("agent-snapshot-memory-warning"),
  ).toBeVisible();
  expect(heightSamples.at(-1)).toBeGreaterThan(initialHeight);
  expect(
    new Set(heightSamples.map((height) => Math.round(height))).size,
  ).toBeGreaterThan(2);
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
    "Duplicate",
    "Share",
    "Managed by team",
  ]);
  await expect(page.getByRole("menuitem", { name: "Edit" })).toHaveCount(0);
  await expect(page.getByRole("menuitem", { name: "Delete" })).toHaveCount(0);
});

test("inactive built-ins cannot be used to create teams", async ({ page }) => {
  await gotoApp(page);

  const error = await invokeTauriExpectError(page, "create_team", {
    input: {
      name: "Honeys",
      personaIds: ["builtin:honey"],
    },
  });

  expect(error).toBe(
    "Honey is not in My Agents. Choose it from Agent Catalog first.",
  );
});

test("built-in removal failures show up from My Agents", async ({ page }) => {
  await gotoApp(page);

  await page.getByTestId("open-agents-view").click();
  await openPersonaCatalog(page);
  await selectCatalogPersona(page, "builtin:honey");
  await useCatalogPersona(page, "builtin:honey");

  await invokeTauri(page, "create_team", {
    input: {
      name: "Honeys",
      personaIds: ["builtin:honey"],
    },
  });

  await page.keyboard.press("Escape");
  await page.getByLabel("Open actions for Honey").click();
  await page.getByRole("menuitem", { name: "Delete" }).click();

  await expect(
    page
      .locator("[data-sonner-toast]")
      .filter({ hasText: "Honey is still referenced by a team." }),
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
