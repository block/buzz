import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { TEST_IDENTITIES, installMockBridge } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";

const OUTDIR = "test-results/invites-settings";
const DIRECT_ADD_HEX =
  "ea9b4d7a7a78a3e3729e5568b14d764d4962be0e1f20f749bcf8d9dbbf9a9328";
const DIRECT_ADD_NPUB =
  "npub1a2d567n60z37xu57245tzntkf4yk90swrus0wjdulrvah0u6jv5qusyp60";
const SECOND_DIRECT_ADD_HEX =
  "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SECOND_DIRECT_ADD_NPUB =
  "npub1424242424242424242424242424242424242424242424242424qamrcaj";

test.beforeEach(async ({ page }, testInfo) => {
  await installMockBridge(page, {
    relayRequiresMembership: true,
    relayRole: testInfo.title.includes("admin can add members")
      ? "admin"
      : "owner",
    uploadDescriptors: [
      {
        filename: "community-picnic.png",
        sha256: "1".repeat(64),
        size: 68,
        type: "image/png",
        uploaded: 1_700_000_000,
        url: `http://localhost:3000/media/${"1".repeat(64)}.png`,
      },
    ],
  });
  await page.route("**/api/invites", async (route) => {
    await route.fulfill({
      contentType: "application/json",
      json: {
        code: "community-email-test",
        expires_at: Math.floor(Date.now() / 1000) + 3 * 86_400,
        url: "https://alpha.example.com/invite/community-email-test",
      },
      status: 200,
    });
  });
  await page.route("http://127.0.0.1:54321/media/**", async (route) => {
    await route.fulfill({
      body: '<svg xmlns="http://www.w3.org/2000/svg" width="320" height="180"><rect width="320" height="180" rx="16" fill="#86efac"/></svg>',
      contentType: "image/svg+xml",
      status: 200,
    });
  });
  await page.route("https://example.com/community-photo.png", async (route) => {
    await route.fulfill({
      body: '<svg xmlns="http://www.w3.org/2000/svg" width="320" height="180"><rect width="320" height="180" rx="16" fill="#93c5fd"/></svg>',
      contentType: "image/svg+xml",
      status: 200,
    });
  });
  await page.goto("/");
  await openSettings(page, "community-members");
});

test("opens a profile from a community member avatar", async ({ page }) => {
  await page.getByRole("button", { name: "Open profile for alice" }).click();

  await expect(page).toHaveURL(
    new RegExp(`/pulse\\?profile=${TEST_IDENTITIES.alice.pubkey}$`),
  );
  await expect(page.getByTestId("user-profile-panel")).toBeVisible();
});

test("capture: consolidated invites settings", async ({ page }) => {
  const panel = page.getByTestId("settings-panel-community-members");

  await expect(
    page.getByTestId("settings-nav-community-members"),
  ).toContainText("Invites");
  await expect(
    page.getByRole("heading", { name: "Invites", exact: true }),
  ).toBeVisible();
  await expect(page.getByTestId("community-icon-settings")).toHaveCount(0);
  await expect(
    page.getByTestId("community-invite-dialog-trigger"),
  ).toBeVisible();
  await expect(page.getByTestId("community-invite-email-field")).toHaveCount(0);
  await expect(page.getByTestId("copy-invite-link")).toHaveCount(0);
  await expect(page.getByText("alice", { exact: true })).toBeVisible();
  await expect(page.getByText("bob", { exact: true })).toBeVisible();
  await expect(
    page.getByText("Manage roles or remove access.", { exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByText("People who use the link join as members."),
  ).toHaveCount(0);
  await expect(page.getByTestId("community-icon-save")).toHaveCount(0);

  const aliceName = page.getByText("alice", { exact: true });
  const aliceRow = page
    .locator('[data-testid^="relay-member-row-"]')
    .filter({ has: aliceName });
  const aliceNpub = aliceRow.locator('[data-testid^="relay-member-npub-"]');
  await expect(aliceName).toHaveCSS("opacity", "1");
  await expect(aliceNpub).toHaveCSS("opacity", "0");
  await aliceRow.hover();
  await expect(aliceName).toHaveCSS("opacity", "0");
  await expect(aliceNpub).toHaveCSS("opacity", "1");
  await page.mouse.move(0, 0);

  await waitForAnimations(page);
  await panel.screenshot({ path: `${OUTDIR}/01-invites-settings.png` });
});

test("builds a personalized welcome sequence", async ({ page }) => {
  const welcomeSettings = page.getByTestId("welcome-channel-settings");
  await expect(welcomeSettings).toBeVisible();
  await welcomeSettings.scrollIntoViewIfNeeded();
  const welcomeRow = welcomeSettings.getByTestId("welcome-channel-row");
  await expect(welcomeRow).toContainText("Custom welcome message");
  await expect(
    welcomeRow.getByRole("button", { name: "Create" }),
  ).toBeVisible();

  await welcomeRow.getByRole("button", { name: "Create" }).click();
  const builder = page.getByTestId("welcome-message-builder");
  await expect(builder).toBeVisible();
  await builder.getByRole("button", { name: "Get writing help" }).click();
  await page
    .getByLabel("What should your welcome message do?")
    .fill(
      "Welcome people by name, share our guide and photo, and point them to introductions.",
    );
  await waitForAnimations(page);
  await builder.screenshot({
    path: `${OUTDIR}/02-welcome-channel-assist.png`,
  });
  const createDraft = builder.getByRole("button", { name: "Create draft" });
  await createDraft.click();
  await expect(createDraft).toHaveAttribute("aria-busy", "true");
  await expect(
    createDraft.getByTestId("welcome-generation-spinner"),
  ).toBeVisible();

  await expect(builder.getByText("{{member}}", { exact: true })).toHaveCount(0);
  await expect(builder.getByTestId("welcome-inline-message")).toContainText(
    "New member",
  );
  const editor = builder.getByTestId("welcome-inline-message");
  await editor.click({ button: "right", position: { x: 420, y: 22 } });
  const linkMenuItem = page.getByRole("menuitem", { name: "Link" });
  await expect(linkMenuItem).toBeVisible();
  const restingMenuItemColor = await linkMenuItem.evaluate(
    (element) => getComputedStyle(element).backgroundColor,
  );
  await linkMenuItem.hover();
  await expect
    .poll(() =>
      linkMenuItem.evaluate(
        (element) => getComputedStyle(element).backgroundColor,
      ),
    )
    .not.toBe(restingMenuItemColor);
  await waitForAnimations(page);
  await builder.screenshot({
    path: `${OUTDIR}/03-welcome-channel-builder.png`,
  });
  await page.keyboard.press("Escape");

  await editor.evaluate((element) => {
    const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT);
    while (walker.nextNode()) {
      const node = walker.currentNode;
      const text = node.textContent ?? "";
      const start = text.indexOf("We’re glad you’re here");
      if (start < 0) continue;
      const range = document.createRange();
      range.setStart(node, start);
      range.setEnd(node, start + "We’re glad you’re here".length);
      const selection = document.getSelection();
      selection?.removeAllRanges();
      selection?.addRange(range);
      element.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
      return;
    }
    throw new Error("Welcome message formatting target was not found");
  });
  const formattingTray = page.getByTestId("welcome-selection-formatting-tray");
  await expect(formattingTray).toBeVisible();
  await waitForAnimations(page);
  await builder.screenshot({
    path: `${OUTDIR}/04-welcome-channel-formatting.png`,
  });
  await formattingTray.getByRole("button", { name: "Bold" }).click();
  await expect(editor.locator("b, strong")).toContainText(
    "We’re glad you’re here",
  );
  const editTypography = await editor.evaluate((element) => {
    const styles = getComputedStyle(element);
    return { fontSize: styles.fontSize, lineHeight: styles.lineHeight };
  });
  const editCanvasLayout = await editor.evaluate((element) => {
    const canvas = element.parentElement?.parentElement;
    if (!canvas) throw new Error("Welcome editor canvas was not found");
    const styles = getComputedStyle(canvas);
    const rect = canvas.getBoundingClientRect();
    return {
      left: rect.left,
      paddingLeft: styles.paddingLeft,
      paddingTop: styles.paddingTop,
      top: rect.top,
    };
  });

  await builder.getByRole("button", { name: /preview/i }).click();
  const preview = page.getByTestId("welcome-channel-preview");
  await expect(preview).toContainText("Welcome, Alex!");
  await expect
    .poll(() =>
      preview.evaluate((element) => {
        const styles = getComputedStyle(element);
        return { fontSize: styles.fontSize, lineHeight: styles.lineHeight };
      }),
    )
    .toEqual(editTypography);
  await expect
    .poll(() =>
      preview.evaluate((element) => {
        const styles = getComputedStyle(element);
        const rect = element.getBoundingClientRect();
        return {
          left: rect.left,
          paddingLeft: styles.paddingLeft,
          paddingTop: styles.paddingTop,
          top: rect.top,
        };
      }),
    )
    .toEqual(editCanvasLayout);
  await expect(preview.locator("b, strong")).toContainText(
    "We’re glad you’re here",
  );
  const previewChipRhythm = await preview
    .locator(".mention-chip")
    .first()
    .evaluate((chip) => {
      const styles = getComputedStyle(chip);
      const message = chip.closest(".message-markdown");
      if (!message) throw new Error("Preview chip is outside message Markdown");
      const probe = document.createElement("span");
      probe.style.position = "absolute";
      probe.style.width = "var(--inline-chip-padding-inline)";
      message.append(probe);
      const sharedPadding = getComputedStyle(probe).width;
      probe.remove();
      return {
        marginLeft: styles.marginLeft,
        marginRight: styles.marginRight,
        paddingRight: styles.paddingRight,
        sharedPadding,
      };
    });
  expect(previewChipRhythm).toEqual({
    marginLeft: "0px",
    marginRight: "0px",
    paddingRight: previewChipRhythm.sharedPadding,
    sharedPadding: previewChipRhythm.sharedPadding,
  });
  await waitForAnimations(page);
  await builder.screenshot({
    path: `${OUTDIR}/05-welcome-channel-preview.png`,
  });
  await builder.getByRole("button", { name: /edit/i }).click();

  await editor.evaluate((element) => {
    const transfer = new DataTransfer();
    transfer.items.add(
      new File(["community picnic"], "community-picnic.png", {
        type: "image/png",
      }),
    );
    const rect = element.getBoundingClientRect();
    element.dispatchEvent(
      new DragEvent("dragenter", {
        bubbles: true,
        clientX: rect.left + 120,
        clientY: rect.top + 22,
        dataTransfer: transfer,
      }),
    );
    element.dispatchEvent(
      new DragEvent("dragover", {
        bubbles: true,
        cancelable: true,
        clientX: rect.left + 120,
        clientY: rect.top + 22,
        dataTransfer: transfer,
      }),
    );
  });
  await expect(builder.getByText("Drop image here")).toBeVisible();
  const dropCaret = page.getByTestId("welcome-image-drop-caret");
  await expect(dropCaret).toBeVisible();
  const initialDropCaret = await dropCaret.boundingBox();

  await editor.evaluate((element) => {
    const transfer = new DataTransfer();
    transfer.items.add(
      new File(["community picnic"], "community-picnic.png", {
        type: "image/png",
      }),
    );
    const rect = element.getBoundingClientRect();
    element.dispatchEvent(
      new DragEvent("dragover", {
        bubbles: true,
        cancelable: true,
        clientX: rect.left + Math.min(620, rect.width - 16),
        clientY: rect.top + 22,
        dataTransfer: transfer,
      }),
    );
  });
  await expect
    .poll(async () => {
      const currentDropCaret = await dropCaret.boundingBox();
      return `${currentDropCaret?.x}:${currentDropCaret?.y}`;
    })
    .not.toBe(`${initialDropCaret?.x}:${initialDropCaret?.y}`);

  await editor.evaluate((element) => {
    const transfer = new DataTransfer();
    transfer.items.add(
      new File(["community picnic"], "community-picnic.png", {
        type: "image/png",
      }),
    );
    const rect = element.getBoundingClientRect();
    element.dispatchEvent(
      new DragEvent("drop", {
        bubbles: true,
        clientX: rect.left + Math.min(620, rect.width - 16),
        clientY: rect.top + 22,
        dataTransfer: transfer,
      }),
    );
  });
  const droppedImage = builder.getByText("community-picnic.png", {
    exact: true,
  });
  await expect(droppedImage).toBeVisible();
  await droppedImage.click();
  await expect(page.getByLabel("Image source")).not.toHaveValue("");
  await builder.getByRole("button", { name: "Close chip editor" }).click();

  await builder.getByRole("button", { name: /preview/i }).click();
  const imagePreview = page
    .getByTestId("welcome-channel-preview")
    .getByRole("img", { name: "community-picnic.png" });
  await expect(imagePreview).toBeVisible();
  await expect(imagePreview).toHaveAttribute(
    "src",
    /^http:\/\/127\.0\.0\.1:54321\/media\//,
  );
  await expect
    .poll(() => imagePreview.evaluate((image) => image.naturalWidth))
    .toBeGreaterThan(0);
  await expect(
    page
      .getByTestId("welcome-channel-preview")
      .getByText("community-picnic.png", { exact: true }),
  ).toHaveCount(0);
  await expect(
    page
      .getByTestId("welcome-channel-preview")
      .locator(".mention-chip")
      .filter({ hasText: "community-picnic.png" }),
  ).toHaveCount(0);
  await waitForAnimations(page);
  await builder.screenshot({
    path: `${OUTDIR}/06-welcome-channel-image-preview.png`,
  });

  await expect(builder.getByRole("button", { name: "Save" })).toBeEnabled();
  await builder.getByRole("button", { name: "Save" }).click();
  await expect(builder).not.toBeVisible();
  await expect(welcomeRow.getByRole("button", { name: "Edit" })).toBeVisible();
  await welcomeRow.getByRole("button", { name: "Edit" }).click();
  await expect(
    builder.getByRole("heading", { name: "Custom welcome message" }),
  ).toBeVisible();
  await expect(builder.getByRole("button", { name: "Save" })).toBeDisabled();
});

test("channel inserts search the community channel list", async ({ page }) => {
  const welcomeRow = page.getByTestId("welcome-channel-row");
  await welcomeRow.scrollIntoViewIfNeeded();
  await welcomeRow.getByRole("button", { name: "Create" }).click();

  const builder = page.getByTestId("welcome-message-builder");
  const editor = builder.getByTestId("welcome-inline-message");
  await editor.click({ button: "right", position: { x: 100, y: 22 } });
  await page.getByRole("menuitem", { name: "Channel" }).click();

  const channelEditor = page.getByRole("dialog", { name: "Edit channel" });
  const channelSearch = channelEditor.getByRole("combobox", {
    name: "Search channels",
  });
  await expect(channelSearch).toBeVisible();
  await expect(page.getByLabel("Channel name")).toHaveCount(0);
  await expect(page.getByLabel("Channel destination")).toHaveCount(0);
  await expect(channelEditor.getByRole("listbox")).toHaveCount(0);

  const pickerBeforeSearch = await channelEditor.boundingBox();

  await channelSearch.fill("rand");
  const pickerWithResults = await channelEditor.boundingBox();
  expect(pickerWithResults?.height).toBeCloseTo(
    pickerBeforeSearch?.height ?? 0,
    0,
  );
  const randomResult = channelEditor.getByRole("option", {
    name: /^random/,
  });
  await expect(randomResult).toBeVisible();
  await waitForAnimations(page);
  await builder.screenshot({
    path: `${OUTDIR}/07-welcome-channel-search.png`,
  });
  await randomResult.click();

  const randomChip = editor.locator("[data-insert-id]", { hasText: "random" });
  await expect(randomChip).toBeVisible();
  await randomChip.click();
  await expect(
    page
      .getByRole("dialog", { name: "Edit channel" })
      .getByRole("option", { name: /^random/ }),
  ).toHaveAttribute("aria-selected", "true");
});

test("capture: share-style community invite dialog", async ({ page }) => {
  await page.getByTestId("community-invite-dialog-trigger").click();

  const dialog = page.getByTestId("community-invite-dialog");
  await expect(dialog).toBeVisible();
  await expect(
    dialog.getByRole("heading", { name: "Invite to community" }),
  ).toBeVisible();
  await expect(page.getByTestId("community-invite-email-field")).toHaveCount(0);
  await expect(page.getByPlaceholder("Type an email address")).toHaveCount(0);
  await expect(
    dialog.getByText(
      "Add someone directly or share a link they can use to join.",
    ),
  ).toBeVisible();
  await expect(
    dialog.getByRole("heading", { name: "Add someone", exact: true }),
  ).toHaveCount(0);
  await expect(dialog.getByTestId("invite-options-divider")).toBeVisible();
  await expect(
    dialog.getByText("Or, copy a link", { exact: true }),
  ).toBeVisible();
  await expect(dialog.getByText("Link settings", { exact: true })).toHaveCount(
    0,
  );
  await expect(page.getByTestId("member-pubkey-input")).toBeVisible();
  await expect(page.getByTestId("member-role")).toHaveCount(0);
  await expect(page.getByTestId("confirm-add-member")).toHaveCount(0);
  await expect(page.getByTestId("invite-link-url")).toHaveValue(
    "https://alpha.example.com/invite/community-email-test",
  );
  await expect(page.getByTestId("copy-invite-link")).toHaveText("Copy link");
  await expect(page.getByTestId("invite-link-ttl-trigger")).toHaveText(
    "3 days",
  );

  await page.getByTestId("member-pubkey-input").fill(DIRECT_ADD_NPUB);
  await expect(page.getByTestId("member-search-popover")).toBeVisible();
  await page.getByTestId(`member-search-result-${DIRECT_ADD_HEX}`).click();
  const memberRole = page.getByTestId("member-role");
  const selectedChip = page.getByTestId(
    `member-search-selection-remove-${DIRECT_ADD_HEX}`,
  );
  await expect(memberRole).toHaveText("Member");
  const inviteButton = page.getByTestId("confirm-add-member");
  await expect(inviteButton).toHaveText("Invite");
  await waitForAnimations(page);
  await expect(inviteButton).toHaveCSS("height", "44px");
  await expect(inviteButton).toHaveJSProperty(
    "offsetHeight",
    await page
      .getByTestId("member-recipient-field")
      .evaluate((field) => Math.round(field.getBoundingClientRect().height)),
  );
  const selectedChipRemoveIcon = selectedChip.locator("span.absolute");
  await expect(selectedChipRemoveIcon).toHaveCSS("opacity", "0");
  await selectedChip.hover();
  await expect(selectedChipRemoveIcon).toHaveCSS("opacity", "1");
  const memberSearch = page.getByTestId("member-pubkey-input");
  await expect(memberSearch).toBeFocused();
  await memberSearch.fill(SECOND_DIRECT_ADD_NPUB);
  await expect(page.getByTestId("member-search-popover")).toBeVisible();
  await page
    .getByTestId(`member-search-result-${SECOND_DIRECT_ADD_HEX}`)
    .click();
  await expect(
    page.getByTestId(`member-search-selection-remove-${SECOND_DIRECT_ADD_HEX}`),
  ).toBeVisible();
  await expect(memberSearch).toBeFocused();
  await memberRole.click();
  await expect(
    page.getByRole("menuitemradio", { name: "Admin" }),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("confirm-add-member")).toBeEnabled();
  await waitForAnimations(page);
  await page.mouse.move(0, 0);
  await dialog.screenshot({ path: `${OUTDIR}/02-invite-dialog.png` });
});

test("admin can add members but cannot assign the admin role", async ({
  page,
}) => {
  await page.getByTestId("community-invite-dialog-trigger").click();

  await page.getByTestId("member-pubkey-input").fill(DIRECT_ADD_NPUB);
  await page.getByTestId(`member-search-result-${DIRECT_ADD_HEX}`).click();
  const memberRole = page.getByTestId("member-role");
  await expect(memberRole).toHaveText("Member");
  await memberRole.click();
  await expect(page.getByRole("menuitemradio", { name: "Admin" })).toHaveCount(
    0,
  );
  await page.keyboard.press("Escape");
});

test("owner can add multiple admins directly by npub from live Invites UI", async ({
  page,
}) => {
  await page.getByTestId("community-invite-dialog-trigger").click();
  await page.getByTestId("member-pubkey-input").fill(DIRECT_ADD_NPUB);
  await page.getByTestId(`member-search-result-${DIRECT_ADD_HEX}`).click();
  await page.getByTestId("member-pubkey-input").fill(SECOND_DIRECT_ADD_NPUB);
  await page
    .getByTestId(`member-search-result-${SECOND_DIRECT_ADD_HEX}`)
    .click();
  await page.getByTestId("member-role").click();
  await page.getByRole("menuitemradio", { name: "Admin" }).click();
  await page.getByTestId("confirm-add-member").click();

  await expect
    .poll(async () =>
      page.evaluate(
        ({ targetPubkeys, role }) =>
          targetPubkeys.every((targetPubkey) =>
            (window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? []).some((entry) => {
              if (entry.command !== "plugin:websocket|send") return false;
              const wireMessage = (
                entry.payload as {
                  message?: { data?: unknown };
                }
              )?.message?.data;
              if (typeof wireMessage !== "string") return false;
              const message = JSON.parse(wireMessage) as unknown[];
              if (message[0] !== "EVENT") return false;
              const event = message[1] as
                | { kind?: number; tags?: string[][] }
                | undefined;
              return (
                event?.kind === 9030 &&
                event.tags?.some(
                  (tag) => tag[0] === "p" && tag[1] === targetPubkey,
                ) &&
                event.tags.some((tag) => tag[0] === "role" && tag[1] === role)
              );
            }),
          ),
        {
          targetPubkeys: [DIRECT_ADD_HEX, SECOND_DIRECT_ADD_HEX],
          role: "admin",
        },
      ),
    )
    .toBe(true);
});
