import { expect, test, type Page } from "@playwright/test";

import {
  createMockAgentMemoryListing,
  installMockBridge,
} from "../helpers/bridge";
import { openProfileMenu, openSettings } from "../helpers/settings";

async function expectHomeView(page: import("@playwright/test").Page) {
  await expect(page.getByTestId("home-inbox-list")).toBeVisible();
}

async function expandIdentity(page: import("@playwright/test").Page) {
  const identity = page.getByTestId("profile-identity-card");
  const isOpen = await identity.evaluate(
    (element) => element instanceof HTMLDetailsElement && element.open,
  );
  if (!isOpen) {
    await page.getByTestId("profile-identity-toggle").click();
  }
}

async function selectFirstEmojiFromPicker(page: Page) {
  const picker = page.locator("em-emoji-picker");
  await expect(picker).toBeVisible();
  await expect
    .poll(() =>
      picker.evaluate((element) =>
        Boolean(element.shadowRoot?.querySelector(".scroll button")),
      ),
    )
    .toBe(true);
  await picker.evaluate((element) => {
    const button = element.shadowRoot?.querySelector(".scroll button");
    if (!(button instanceof HTMLElement)) {
      throw new Error("Emoji picker did not render an emoji button.");
    }
    button.click();
  });
}

async function waitForAvatarEditorToClose(page: Page) {
  await expect(page.getByTestId("profile-avatar-editor-shell")).toHaveCount(0);
}

async function waitForReactEffects(page: Page) {
  await page.evaluate(
    () =>
      new Promise<void>((resolve) => {
        requestAnimationFrame(() => {
          requestAnimationFrame(resolve);
        });
      }),
  );
}

async function addGenericAgent(
  page: Page,
  channelName: string,
  agentName: string,
): Promise<string> {
  await page.getByTestId(`channel-${channelName}`).click();
  await expect(page.getByTestId("chat-title")).toHaveText(channelName);
  const channelId = await page
    .getByTestId(`channel-${channelName}`)
    .getAttribute("data-channel-id");
  if (!channelId) {
    throw new Error(`Channel ${channelName} is missing a data-channel-id.`);
  }

  await page.waitForFunction(() => {
    return Boolean(
      (
        window as Window & {
          __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: unknown;
        }
      ).__BUZZ_E2E_INVOKE_MOCK_COMMAND__,
    );
  });
  return page.evaluate(
    async ({ agentName, channelId }) => {
      const invoke = (
        window as Window & {
          __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
            command: string,
            payload?: Record<string, unknown>,
          ) => Promise<{ agent?: { pubkey: string } }>;
        }
      ).__BUZZ_E2E_INVOKE_MOCK_COMMAND__;
      if (!invoke) {
        throw new Error("Mock bridge is not installed.");
      }

      const created = await invoke("create_managed_agent", {
        input: {
          name: agentName,
          spawnAfterCreate: true,
          systemPrompt: "Watch the channel and help when asked.",
        },
      });
      const pubkey = created.agent?.pubkey;
      if (!pubkey) {
        throw new Error("Mock managed agent creation did not return a pubkey.");
      }

      await invoke("add_channel_members", {
        channelId,
        pubkeys: [pubkey],
        role: "bot",
      });

      await (
        window as Window & {
          __BUZZ_E2E_QUERY_CLIENT__?: {
            invalidateQueries: () => Promise<void>;
          };
        }
      ).__BUZZ_E2E_QUERY_CLIENT__?.invalidateQueries();

      return pubkey;
    },
    { agentName, channelId },
  );
}

async function waitForMockLiveSubscription(page: Page, channelName: string) {
  await expect
    .poll(async () => {
      return page.evaluate((channelName) => {
        return (
          (
            window as Window & {
              __BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
                channelName: string;
              }) => boolean;
            }
          ).__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({ channelName }) ?? false
        );
      }, channelName);
    })
    .toBe(true);
}

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

test("updates the relay-backed profile from settings", async ({ page }) => {
  const stamp = Date.now();
  const displayName = `Tyler QA ${stamp}`;
  const avatarUrl = `https://example.com/avatar-${stamp}.png`;
  const about = `Coordinating relay profile setup ${stamp}`;
  await page.goto("/");

  await openSettings(page, "profile");
  await expect(
    page.getByTestId("settings-profile").getByRole("heading", {
      exact: true,
      name: "Profile",
    }),
  ).toBeVisible();

  await expect(page.getByTestId("profile-identity-details")).toBeHidden();
  await expandIdentity(page);
  await expect(page.getByTestId("profile-pubkey")).toContainText("deadbeef");
  await expect(page.getByTestId("profile-nip05")).toContainText("Not set");

  await page.getByTestId("profile-metadata-edit").click();
  await expect(page.getByTestId("profile-metadata-edit")).toHaveText("Done");
  await expect(page.getByTestId("profile-about")).toBeVisible();
  await page.getByTestId("profile-display-name").fill(displayName);
  await page.getByTestId("profile-about").fill(about);
  await page.getByTestId("profile-metadata-edit").click();

  await expect(page.getByTestId("profile-display-name-value")).toHaveText(
    displayName,
  );
  await expect(page.getByTestId("profile-about-value")).toHaveText(about);

  await page.getByTestId("profile-avatar-edit").click();
  await page.getByTestId("profile-avatar-url").fill(avatarUrl);
  await page.getByTestId("profile-avatar-done").click();
  await waitForAvatarEditorToClose(page);

  await expect(page.getByTestId("profile-display-name-value")).toHaveText(
    displayName,
  );
  await expect(page.getByTestId("profile-nip05")).toContainText("Not set");
  await page.getByTestId("profile-avatar-edit").click();
  await expect(page.getByTestId("profile-avatar-url")).toHaveValue("");
  await page.getByTestId("profile-avatar-done").click();
  await expandIdentity(page);

  await page.getByTestId("settings-back-to-app").click();
  await expectHomeView(page);
  await expect(page.getByTestId("open-settings")).toBeVisible();

  await openSettings(page, "profile");
  await expect(page.getByTestId("profile-display-name-value")).toHaveText(
    displayName,
  );
  await expandIdentity(page);
  await expect(page.getByTestId("profile-nip05")).toContainText("Not set");
  await page.getByTestId("profile-avatar-edit").click();
  await expect(page.getByTestId("profile-avatar-url")).toHaveValue("");
  await expect(page.getByTestId("profile-about-value")).toHaveText(about);
});

test("saves profile metadata from the block Done button", async ({ page }) => {
  await page.goto("/");

  await openSettings(page, "profile");
  await expect(page.getByTestId("profile-display-name-value")).toHaveText(
    "npub1mock...",
  );
  await expect(page.getByTestId("profile-save")).toHaveCount(0);

  await page.getByTestId("profile-metadata-edit").click();
  await expect(page.getByTestId("profile-metadata-edit")).toHaveText("Done");
  await expect(page.getByTestId("profile-about")).toBeVisible();
  await page.getByTestId("profile-display-name").fill("Save Button QA");
  await page.getByTestId("profile-about").fill("Temporary profile note");
  await expect(page.getByTestId("profile-save")).toHaveCount(0);

  await page.getByTestId("profile-metadata-edit").click();
  await waitForReactEffects(page);
  await expect(page.getByTestId("profile-display-name")).toHaveCount(0);
  await expect(page.getByTestId("profile-display-name-value")).toHaveText(
    "Save Button QA",
  );
  await expect(page.getByTestId("profile-about-value")).toHaveText(
    "Temporary profile note",
  );
  await expect(page.getByTestId("profile-metadata-edit")).toHaveText("Edit");
  await expect(page.getByTestId("profile-save")).toHaveCount(0);

  await page.getByTestId("profile-metadata-edit").click();
  await page.getByTestId("profile-about").fill("");
  await page.getByTestId("profile-metadata-edit").click();
  await waitForReactEffects(page);
  await expect(page.getByTestId("profile-about-value")).toHaveText("Not set");
  await expect(page.getByTestId("profile-save")).toHaveCount(0);

  await page.getByTestId("profile-metadata-edit").click();
  await page.getByTestId("profile-display-name").fill("");
  await expect(
    page.getByText("Clearing existing profile fields is not supported yet."),
  ).toBeVisible();
  await page.getByTestId("profile-metadata-edit").click();
  await waitForReactEffects(page);
  await expect(page.getByTestId("profile-display-name")).toHaveCount(0);
  await expect(page.getByTestId("profile-display-name-value")).toHaveText(
    "Save Button QA",
  );
  await expect(page.getByTestId("profile-metadata-edit")).toHaveText("Edit");

  await page.getByTestId("profile-metadata-edit").click();
  await page.getByTestId("profile-display-name").fill("npub1mock...");
  await page.getByTestId("profile-metadata-edit").click();
  await expect(page.getByTestId("profile-save")).toHaveCount(0);
});

test("shows profile save feedback as a toast", async ({ page }) => {
  await page.goto("/");

  await openSettings(page, "profile");
  await page.getByTestId("profile-metadata-edit").click();
  await page.getByTestId("profile-display-name").fill("Toast QA");
  await page.getByTestId("profile-metadata-edit").click();

  await expect(
    page.locator("[data-sonner-toast]").filter({ hasText: "Profile saved" }),
  ).toBeVisible();
  await expect(page.getByText("Profile saved.", { exact: true })).toHaveCount(
    0,
  );
});

test("nests the avatar edit button in a clipped notch", async ({ page }) => {
  await page.goto("/");

  await openSettings(page, "profile");

  await expect(page.getByTestId("profile-avatar-preview-clip")).toHaveCSS(
    "clip-path",
    /url/,
  );
  const editShell = page.getByTestId("profile-avatar-edit-shell");
  await expect(editShell).toHaveCSS("height", "54px");
  await expect(editShell).toHaveCSS("width", "54px");

  const editButton = page.getByTestId("profile-avatar-edit");
  await expect(editButton).toHaveCSS("opacity", "1");

  await expect(editButton).toHaveCSS(
    "background-color",
    await page
      .getByTestId("settings-nav-profile")
      .evaluate((element) => getComputedStyle(element).backgroundColor),
  );
  const transitionProperty = await editButton.evaluate(
    (element) => getComputedStyle(element).transitionProperty,
  );
  expect(transitionProperty).toContain("opacity");
  expect(transitionProperty).toContain("scale");
});

test("swaps the avatar preview and mode tabs while editing", async ({
  page,
}) => {
  await page.goto("/");

  await openSettings(page, "profile");

  const previewFrame = page.getByTestId("profile-avatar-clip-frame");
  const closedPreviewBox = await previewFrame.boundingBox();
  if (!closedPreviewBox) {
    throw new Error("Profile avatar preview did not render bounds.");
  }

  await page.getByTestId("profile-avatar-edit").click();
  const tabList = page.getByRole("tablist", { name: "Avatar type" });
  await expect(tabList).toBeVisible();
  await expect(page.getByTestId("profile-avatar-mode-tabs-slot")).toBeVisible();
  await page.waitForTimeout(350);

  const openPreviewBox = await previewFrame.boundingBox();
  const tabListBox = await tabList.boundingBox();
  if (!openPreviewBox || !tabListBox) {
    throw new Error("Profile avatar edit layout did not render bounds.");
  }

  const closedPreviewCenterY = closedPreviewBox.y + closedPreviewBox.height / 2;
  const tabListCenterY = tabListBox.y + tabListBox.height / 2;
  expect(Math.abs(tabListCenterY - closedPreviewCenterY)).toBeLessThan(16);
  const tabListBottomY = tabListBox.y + tabListBox.height;
  const segmentToPreviewGap = openPreviewBox.y - tabListBottomY;
  expect(segmentToPreviewGap).toBeGreaterThan(48);
  expect(segmentToPreviewGap).toBeLessThan(72);
  expect(openPreviewBox.y).toBeGreaterThan(closedPreviewCenterY + 72);

  await page.getByTestId("profile-avatar-done").click();
  await waitForAvatarEditorToClose(page);
  await expect(tabList).toHaveCount(0);

  const restoredPreviewBox = await previewFrame.boundingBox();
  if (!restoredPreviewBox) {
    throw new Error("Profile avatar preview did not restore bounds.");
  }
  expect(Math.abs(restoredPreviewBox.y - closedPreviewBox.y)).toBeLessThan(8);
});

test("highlights the avatar drop target while dragging an image", async ({
  page,
}) => {
  await page.goto("/");

  await openSettings(page, "profile");
  await page.getByTestId("profile-avatar-edit").click();

  const uploadTarget = page.getByTestId("profile-avatar-upload");
  await uploadTarget.evaluate((element) => {
    const dataTransfer = new DataTransfer();
    dataTransfer.items.add(
      new File(["avatar"], "avatar.png", { type: "image/png" }),
    );

    element.dispatchEvent(
      new DragEvent("dragenter", {
        bubbles: true,
        cancelable: true,
        dataTransfer,
      }),
    );
  });

  await expect(uploadTarget).toHaveAttribute("data-dragging", "true");
  await expect(uploadTarget).toContainText("Drop image here");

  await uploadTarget.evaluate((element) => {
    const dataTransfer = new DataTransfer();
    dataTransfer.items.add(
      new File(["avatar"], "avatar.png", { type: "image/png" }),
    );

    element.dispatchEvent(
      new DragEvent("dragleave", {
        bubbles: true,
        cancelable: true,
        dataTransfer,
      }),
    );
  });

  await expect(uploadTarget).not.toHaveAttribute("data-dragging", "true");

  await uploadTarget.evaluate((element) => {
    const dataTransfer = new DataTransfer();
    dataTransfer.items.add(
      new File(["avatar"], "avatar.png", { type: "image/png" }),
    );

    element.dispatchEvent(
      new DragEvent("dragenter", {
        bubbles: true,
        cancelable: true,
        dataTransfer,
      }),
    );
  });

  await expect(uploadTarget).toHaveAttribute("data-dragging", "true");

  await page.evaluate(() => {
    window.dispatchEvent(
      new DragEvent("dragleave", {
        bubbles: true,
        cancelable: true,
        clientX: -1,
        clientY: 40,
      }),
    );
  });

  await expect(uploadTarget).not.toHaveAttribute("data-dragging", "true");
});

test("uploads local profile avatar files before saving", async ({ page }) => {
  const uploadedAvatarUrl = "https://mock.relay/media/avatar-profile.png";
  await installMockBridge(page, {
    uploadDescriptors: [
      {
        filename: "avatar-profile.png",
        sha256: "b".repeat(64),
        size: 553432,
        type: "image/png",
        uploaded: 1_779_900_000,
        url: uploadedAvatarUrl,
      },
    ],
  });
  await page.goto("/");

  await openSettings(page, "profile");
  await page.getByTestId("profile-avatar-edit").click();
  await page.getByTestId("profile-avatar-input").setInputFiles({
    buffer: Buffer.from("large-avatar-bytes"),
    mimeType: "image/png",
    name: "avatar-profile.png",
  });

  await expect(page.getByTestId("profile-avatar-url")).toHaveValue("");
  await page.getByTestId("profile-avatar-done").click();
  await waitForAvatarEditorToClose(page);
  await page.getByTestId("profile-avatar-edit").click();
  await expect(page.getByTestId("profile-avatar-url")).toHaveValue("");

  const pastedAvatarUrl = await page.evaluate(
    () => new URL("/buzz.svg", window.location.href).href,
  );
  await page.getByTestId("profile-avatar-url").click();
  await page.keyboard.insertText(pastedAvatarUrl);
  await expect(page.getByTestId("profile-avatar-url")).toHaveValue(
    pastedAvatarUrl,
  );
  await page.getByTestId("profile-avatar-done").click();
  await waitForAvatarEditorToClose(page);
  await page.getByTestId("profile-avatar-edit").click();
  await expect(page.getByTestId("profile-avatar-url")).toHaveValue("");
  await page.getByTestId("profile-avatar-url").fill("");
  await page.getByTestId("profile-avatar-done").click();
  await expect(
    page.getByTestId("profile-avatar-preview").locator("img"),
  ).toHaveCount(1);
  await waitForAvatarEditorToClose(page);
  await page.getByTestId("profile-avatar-edit").click();
  await expect(page.getByTestId("profile-avatar-url")).toHaveValue("");

  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as Window & { __BUZZ_E2E_COMMANDS__?: string[] })
            .__BUZZ_E2E_COMMANDS__ ?? [],
      ),
    )
    .toEqual(expect.arrayContaining(["upload_media_bytes", "update_profile"]));
});

test("renders emoji avatars with a static background layer", async ({
  page,
}) => {
  await page.goto("/");

  await openSettings(page, "profile");
  await page.getByTestId("profile-avatar-edit").click();
  await page.getByRole("tab", { name: "Emoji" }).click();
  await selectFirstEmojiFromPicker(page);
  await page.getByRole("button", { name: "Use #FFE75C background" }).click();

  const avatarPreview = page.getByTestId("profile-avatar-preview");
  await expect(avatarPreview).toHaveCSS(
    "background-color",
    "rgb(255, 231, 92)",
  );
  await expect(avatarPreview).not.toHaveClass(/buzz-avatar-squish/);
  await expect(page.getByTestId("profile-avatar-preview-emoji")).toHaveText(
    "😀",
  );
  await expect(page.getByTestId("profile-avatar-preview-emoji")).toHaveCSS(
    "font-size",
    "96px",
  );
});

test("reveals emoji background colors only after choosing an emoji", async ({
  page,
}) => {
  const imageAvatarUrl = `https://example.com/avatar-color-controls-${Date.now()}.png`;
  await page.goto("/");

  await openSettings(page, "profile");
  await page.getByTestId("profile-avatar-edit").click();
  await page.getByTestId("profile-avatar-url").fill(imageAvatarUrl);
  await page.getByTestId("profile-avatar-done").click();
  await waitForAvatarEditorToClose(page);

  await page.getByTestId("profile-avatar-edit").click();
  await expect(page.getByTestId("profile-avatar-url")).toHaveValue("");
  await page.getByRole("tab", { name: "Emoji" }).click();

  const colorGridShell = page.getByTestId("profile-avatar-color-grid-shell");
  const doneButton = page.getByTestId("profile-avatar-done");
  await expect(colorGridShell).toHaveAttribute("aria-hidden", "true");

  const doneBeforeEmoji = await doneButton.boundingBox();
  if (!doneBeforeEmoji) {
    throw new Error("Avatar Done button did not render bounds.");
  }

  await selectFirstEmojiFromPicker(page);

  await expect(colorGridShell).toHaveAttribute("aria-hidden", "false");
  await expect(page.getByTestId("profile-avatar-color-grid")).toBeVisible();
  await colorGridShell.evaluate((element) =>
    Promise.all(
      element
        .getAnimations()
        .map((animation) => animation.finished.catch(() => undefined)),
    ),
  );

  const doneAfterEmoji = await doneButton.boundingBox();
  if (!doneAfterEmoji) {
    throw new Error("Avatar Done button did not render bounds.");
  }
  expect(doneAfterEmoji.y).toBeGreaterThan(doneBeforeEmoji.y + 8);
});

test("snaps custom avatar colors to the dot grid", async ({ page }) => {
  await page.goto("/");

  await openSettings(page, "profile");
  await page.getByTestId("profile-avatar-edit").click();
  await page.getByRole("tab", { name: "Emoji" }).click();
  await selectFirstEmojiFromPicker(page);

  const customColorSwatch = page.getByTestId("profile-avatar-custom-color");
  await customColorSwatch.click();

  const spectrum = page.getByTestId("profile-avatar-custom-color-spectrum");
  await expect(spectrum).toBeVisible();
  await expect(page.getByTestId("profile-avatar-done")).toHaveCount(0);
  await expect(
    page.getByTestId("profile-avatar-custom-color-done"),
  ).toBeVisible();

  const hueSlider = page.getByTestId("profile-avatar-custom-color-hue");
  await hueSlider.press("Home");
  await expect(hueSlider).toHaveAttribute("aria-valuenow", "0");

  const spectrumBox = await spectrum.boundingBox();
  if (!spectrumBox) {
    throw new Error("Custom color spectrum did not render bounds.");
  }

  await spectrum.click({
    position: {
      x: 24 + (spectrumBox.width - 48) * 0.33,
      y: 24 + (spectrumBox.height - 48) * 0.44,
    },
  });
  await expect(page.getByTestId("profile-avatar-preview")).toHaveCSS(
    "background-color",
    "rgb(145, 93, 93)",
  );
  await page.getByTestId("profile-avatar-custom-color-done").click();

  await expect(customColorSwatch).toHaveAttribute("aria-pressed", "true");
  await expect(customColorSwatch).toHaveCSS(
    "background-color",
    "rgb(145, 93, 93)",
  );
  await expect(page.getByTestId("profile-avatar-done")).toBeVisible();
});

test("updates presence from the profile menu", async ({ page }) => {
  await page.goto("/");

  await openProfileMenu(page);
  await expect(
    page.getByTestId("profile-popover-current-status"),
  ).toContainText("Online");

  await page.getByTestId("profile-popover-presence-trigger").click();
  await page.getByTestId("profile-popover-status-away").click();
  await openProfileMenu(page);
  await expect(
    page.getByTestId("profile-popover-current-status"),
  ).toContainText("Away");

  await page.getByTestId("profile-popover-presence-trigger").click();
  await page.getByTestId("profile-popover-status-offline").click();
  await openProfileMenu(page);
  await expect(
    page.getByTestId("profile-popover-current-status"),
  ).toContainText("Offline");
});

test("renders agent memories seeded through the Playwright mock bridge", async ({
  page,
}) => {
  await installMockBridge(page, {
    agentMemory: createMockAgentMemoryListing(),
  });
  await page.goto("/");

  const agentPubkey = await addGenericAgent(page, "general", "Memory Bot");

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");

  await page.evaluate(
    ({ pubkey }) => {
      const emit = (
        window as Window & {
          __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
            channelName: string;
            content: string;
            pubkey: string;
          }) => unknown;
        }
      ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
      if (!emit) {
        throw new Error("Mock message emitter is unavailable.");
      }
      emit({
        channelName: "general",
        content: "Memory bot check-in",
        pubkey,
      });
    },
    { pubkey: agentPubkey },
  );

  const messageRow = page
    .getByTestId("message-row")
    .filter({ hasText: "Memory bot check-in" });
  await expect(messageRow).toBeVisible();
  await messageRow.locator("button").first().click();

  await expect(page.getByTestId("user-profile-panel")).toBeVisible();
  const memoriesIngress = page.getByTestId("user-profile-memories-ingress");
  await expect(memoriesIngress).toContainText("Memories");
  await expect(memoriesIngress).toContainText("9");
  await memoriesIngress.click();

  await expect(page.getByTestId("agent-memory-section")).toBeVisible();
  await expect(page.getByTestId("agent-memory-list")).toContainText(
    "ui-density",
  );
  await expect(page.getByTestId("agent-memory-truncated")).toContainText(
    "View all (9)",
  );
  await page.getByTestId("agent-memory-truncated").click();
  await expect(page.getByTestId("agent-memory-list")).toContainText("orphan");
});

test("renders settings in the app shell with a back button", async ({
  page,
}) => {
  await page.goto("/");

  const inboxNavButton = page
    .getByTestId("app-sidebar")
    .getByRole("button", { name: "Inbox" });
  await expect(inboxNavButton).toBeVisible();

  await openSettings(page);
  await expect(page.getByTestId("settings-sidebar")).toBeVisible();
  await expect(page.getByTestId("settings-back-to-app")).toBeVisible();
  await expect(page.getByPlaceholder("Search everything")).toHaveCount(0);
  await expect(page.getByText("Personal", { exact: true })).toBeVisible();
  await expect(page.getByTestId("settings-nav-profile")).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(page.getByText("Workspaces", { exact: true })).toBeVisible();
  await expect(
    page.getByTestId("settings-nav-channel-templates"),
  ).toBeVisible();
  await expect(page.getByText("App", { exact: true })).toBeVisible();
  await expect(page.getByTestId("settings-nav-agents")).toBeVisible();
  await expect(
    page.getByTestId("settings-profile").getByRole("heading", {
      exact: true,
      name: "Profile",
    }),
  ).toBeVisible();
  await page.getByTestId("settings-nav-appearance").click();
  await expect(
    page.getByTestId("settings-theme").getByRole("heading", {
      name: "Appearance",
    }),
  ).toBeVisible();
  await expect(inboxNavButton).toHaveCount(0);

  await page.getByTestId("settings-back-to-app").click();
  await expectHomeView(page);
  await expect(inboxNavButton).toBeVisible();
});

test("notification settings drive the Inbox badge and desktop alerts", async ({
  page,
}) => {
  async function getAppBadgeCount() {
    return page.evaluate(() => {
      const win = window as Window & {
        __BUZZ_E2E_APP_BADGE_COUNT__?: number;
      };

      return win.__BUZZ_E2E_APP_BADGE_COUNT__ ?? 0;
    });
  }

  await page.goto("/");
  await expect(page.getByTestId("sidebar-home-count")).toHaveCount(0);

  await openSettings(page, "notifications");
  await expect(page.getByTestId("settings-notifications")).toBeVisible();
  await expect(page.getByTestId("notifications-desktop-state")).toContainText(
    "On",
  );

  await page.getByTestId("settings-back-to-app").click();
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  // The dock badge sums unreadChannelIds.size + homeBadgeCount. Seeded test
  // channels may start with unreads, so capture the baseline after navigating
  // to general (which marks it read) but before injecting the mock mention.
  const baseline = await getAppBadgeCount();

  await page.evaluate(() => {
    const win = window as Window & {
      __BUZZ_E2E_PUSH_MOCK_FEED_ITEM__?: (item: {
        category: "mention" | "needs_action" | "activity" | "agent_activity";
        channel_id: string | null;
        channel_name: string;
        content: string;
        created_at: number;
        id: string;
        kind: number;
        pubkey: string;
        tags: string[][];
      }) => unknown;
    };

    win.__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__?.({
      category: "mention",
      channel_id: "1c7e1c02-87bb-5e88-b2da-5a7a9432d0c9",
      channel_name: "engineering",
      content: "Please review the rollout checklist.",
      created_at: Math.floor(Date.now() / 1000) + 5,
      id: `mock-feed-notification-${Date.now()}`,
      kind: 9,
      pubkey:
        "bb22a5299220cad76ffd46190ccbeede8ab5dc260faa28b6e5a2cb31b9aff260",
      tags: [
        ["e", "1c7e1c02-87bb-5e88-b2da-5a7a9432d0c9"],
        [
          "p",
          "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
        ],
      ],
    });
  });

  await expect(page.getByTestId("sidebar-home-count")).toHaveText("1");
  await expect.poll(getAppBadgeCount).toBe(baseline + 1);

  await expect
    .poll(() =>
      page.evaluate(() => {
        const win = window as Window & {
          __BUZZ_E2E_NOTIFICATIONS__?: Array<{
            body: string | null;
            title: string;
          }>;
        };

        return win.__BUZZ_E2E_NOTIFICATIONS__?.length ?? 0;
      }),
    )
    .toBe(1);

  const notifications = await page.evaluate(() => {
    const win = window as Window & {
      __BUZZ_E2E_NOTIFICATIONS__?: Array<{
        body: string | null;
        title: string;
      }>;
    };

    return win.__BUZZ_E2E_NOTIFICATIONS__ ?? [];
  });

  expect(notifications).toEqual([
    {
      body: "Please review the rollout checklist.",
      title: "bob mentioned you in #engineering",
    },
  ]);

  const clickedNotification = await page.evaluate(() => {
    const win = window as Window & {
      __BUZZ_E2E_CLICK_NOTIFICATION__?: (index: number) => boolean;
    };

    return win.__BUZZ_E2E_CLICK_NOTIFICATION__?.(0) ?? false;
  });
  expect(clickedNotification).toBe(true);

  await expect(page.getByTestId("chat-title")).toHaveText("engineering");
  await expect(page.getByTestId("message-timeline")).toContainText(
    "Please review the rollout checklist.",
  );

  await openSettings(page, "notifications");
  await page.getByTestId("notifications-home-badge-toggle").click();
  await page.getByTestId("settings-back-to-app").click();
  await expect(page.getByTestId("chat-title")).toHaveText("engineering");
  await expect(page.getByTestId("sidebar-home-count")).toHaveCount(0);
  await expect.poll(getAppBadgeCount).toBe(baseline);

  await openSettings(page, "notifications");
  await page.getByTestId("notifications-home-badge-toggle").click();
  await page.getByTestId("settings-back-to-app").click();
  await expect(page.getByTestId("sidebar-home-count")).toHaveText("1");
  await expect.poll(getAppBadgeCount).toBe(baseline + 1);

  await page
    .getByTestId("app-sidebar")
    .getByRole("button", { name: "Inbox" })
    .click();
  await expectHomeView(page);
  await expect(page.getByTestId("sidebar-home-count")).toHaveCount(0);
  await expect.poll(getAppBadgeCount).toBe(baseline);
});

test("desktop notification clicks open the matching forum thread", async ({
  page,
}) => {
  await page.goto("/");

  await openSettings(page, "notifications");
  await expect(page.getByTestId("notifications-desktop-state")).toContainText(
    "On",
  );
  await page.getByTestId("settings-back-to-app").click();
  await expectHomeView(page);

  await page.evaluate(() => {
    const win = window as Window & {
      __BUZZ_E2E_PUSH_MOCK_FEED_ITEM__?: (item: {
        category: "mention" | "needs_action" | "activity" | "agent_activity";
        channel_id: string | null;
        channel_name: string;
        content: string;
        created_at: number;
        id: string;
        kind: number;
        pubkey: string;
        tags: string[][];
      }) => unknown;
    };

    win.__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__?.({
      category: "mention",
      channel_id: "a27e1ee9-76a6-5bdf-a5d5-1d85610dad11",
      channel_name: "watercooler",
      content: "Release checklist: async feedback thread.",
      created_at: Math.floor(Date.now() / 1000) + 5,
      id: "mock-forum-release-thread",
      kind: 45001,
      pubkey:
        "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f",
      tags: [["h", "a27e1ee9-76a6-5bdf-a5d5-1d85610dad11"]],
    });
  });

  await expect
    .poll(() =>
      page.evaluate(() => {
        const win = window as Window & {
          __BUZZ_E2E_NOTIFICATIONS__?: Array<{
            body: string | null;
            title: string;
          }>;
        };

        return win.__BUZZ_E2E_NOTIFICATIONS__?.length ?? 0;
      }),
    )
    .toBe(1);

  const clickedNotification = await page.evaluate(() => {
    const win = window as Window & {
      __BUZZ_E2E_CLICK_NOTIFICATION__?: (index: number) => boolean;
    };

    return win.__BUZZ_E2E_CLICK_NOTIFICATION__?.(0) ?? false;
  });
  expect(clickedNotification).toBe(true);

  await expect(page.getByTestId("chat-title")).toHaveText("watercooler");
  await expect(
    page.getByRole("button", { name: "Back to posts" }),
  ).toBeVisible();
  await expect(
    page.getByText("Release checklist: async feedback thread."),
  ).toBeVisible();
});

test("opens settings with the keyboard shortcut and updates theme", async ({
  page,
}) => {
  await page.goto("/");
  await expectHomeView(page);

  await page.keyboard.press(
    process.platform === "darwin" ? "Meta+," : "Control+,",
  );

  await expect(page.getByTestId("settings-view")).toBeVisible();
  await expect(page.getByTestId("settings-nav-profile")).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(
    page.getByTestId("settings-profile").getByRole("heading", {
      exact: true,
      name: "Profile",
    }),
  ).toBeVisible();
  await page.getByTestId("settings-nav-appearance").click();

  // Default theme is catppuccin-macchiato (dark)
  await expect
    .poll(() =>
      page.evaluate(() => document.documentElement.classList.contains("dark")),
    )
    .toBe(true);

  // Switch to a light theme — verifies dark→light transition
  await page.getByTestId("theme-option-github-light").click();

  await expect
    .poll(() =>
      page.evaluate(() => document.documentElement.classList.contains("light")),
    )
    .toBe(true);

  await expect
    .poll(() =>
      page.evaluate(() => document.documentElement.classList.contains("dark")),
    )
    .toBe(false);

  // CSS variables are set on the root element (the real theming mechanism)
  await expect
    .poll(() =>
      page.evaluate(() =>
        document.documentElement.style.getPropertyValue("--background").trim(),
      ),
    )
    .toBeTruthy();

  // Theme name persists in localStorage
  await expect
    .poll(() => page.evaluate(() => localStorage.getItem("buzz-theme")))
    .toBe("github-light");

  // Switch back to a dark theme — verifies light→dark transition
  await page.getByTestId("theme-option-dracula").click();

  await expect
    .poll(() =>
      page.evaluate(() => document.documentElement.classList.contains("dark")),
    )
    .toBe(true);

  await expect
    .poll(() => page.evaluate(() => localStorage.getItem("buzz-theme")))
    .toBe("dracula");

  // Close settings with keyboard shortcut
  await page.keyboard.press(
    process.platform === "darwin" ? "Meta+," : "Control+,",
  );
  await expect(page.getByTestId("settings-view")).toHaveCount(0);
  await expectHomeView(page);
});

test("supports webview zoom keyboard shortcuts", async ({ page }) => {
  await page.goto("/");
  await expectHomeView(page);

  const getTextScaleState = () =>
    page.evaluate(() => ({
      fontSize: getComputedStyle(document.documentElement).fontSize,
      storedScale: localStorage.getItem("buzz:text-scale"),
      webviewZoom: window.__BUZZ_E2E_WEBVIEW_ZOOM__,
    }));
  const dispatchPrimaryShortcut = (
    key: string,
    code: string,
    shiftKey = false,
  ) =>
    page.evaluate(
      ({ code, key, shiftKey }) => {
        const isMac = /mac|iphone|ipad|ipod/i.test(navigator.platform);
        window.dispatchEvent(
          new KeyboardEvent("keydown", {
            bubbles: true,
            cancelable: true,
            code,
            ctrlKey: !isMac,
            key,
            metaKey: isMac,
            shiftKey,
          }),
        );
      },
      { code, key, shiftKey },
    );

  await dispatchPrimaryShortcut("+", "Equal", true);

  await expect.poll(getTextScaleState).toEqual({
    fontSize: "17.6px",
    storedScale: "1.1",
    webviewZoom: 1,
  });

  await dispatchPrimaryShortcut("-", "Minus");

  await expect.poll(getTextScaleState).toEqual({
    fontSize: "16px",
    storedScale: null,
    webviewZoom: 1,
  });

  await dispatchPrimaryShortcut("+", "Equal", true);
  await dispatchPrimaryShortcut("+", "Equal", true);

  await expect.poll(getTextScaleState).toEqual({
    fontSize: "19.2px",
    storedScale: "1.2",
    webviewZoom: 1,
  });

  await dispatchPrimaryShortcut("0", "Digit0");

  await expect.poll(getTextScaleState).toEqual({
    fontSize: "16px",
    storedScale: null,
    webviewZoom: 1,
  });
});

test("shows doctor checks for local CLI tooling", async ({ page }) => {
  await page.goto("/");

  await openSettings(page, "doctor");

  await expect(page.getByTestId("settings-doctor")).toBeVisible();
  await expect(page.getByTestId("doctor-runtime-goose")).toContainText("Goose");
});
