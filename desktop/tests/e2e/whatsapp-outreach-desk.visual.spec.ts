import { expect, test, type Page } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import path from "node:path";

import { waitForAnimations } from "../helpers/animations";
import {
  OUTREACH_CHANNEL_ID,
  gotoWhatsAppScreen,
  installWhatsAppMockBridge,
  type WhatsAppProofTheme,
} from "../helpers/whatsappOutreach";

type WhatsAppEventWindow = Window & {
  __BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
    channelName: string;
  }) => boolean;
  __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
    channelName: string;
    content: string;
    pubkey: string;
    kind: number;
    parentEventId?: string;
    extraTags?: string[][];
    createdAt?: number;
  }) => { id: string };
};

async function installLogoPreview(page: Page) {
  await page.route("https://img.logo.dev/**", (route) =>
    route.fulfill({
      status: 200,
      contentType: "image/svg+xml",
      body: '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect width="64" height="64" rx="14" fill="#3d664b"/><path d="M14 42h36v5H14zm4-7 14-18 14 18z" fill="#f5f2e7"/></svg>',
    }),
  );
}

async function emitDiscussionFixture(page: Page) {
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (
            window as WhatsAppEventWindow
          ).__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
            channelName: "Outreach",
          }) ?? false,
      ),
    )
    .toBe(true);

  const emit = async (input: {
    content: string;
    pubkey?: string;
    tags?: string[][];
    parentEventId?: string;
  }) => {
    const event = await page.evaluate(
      ({ content, pubkey, tags, parentEventId }) =>
        (window as WhatsAppEventWindow).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
          channelName: "Outreach",
          content,
          pubkey: pubkey ?? "deadbeef".repeat(8),
          kind: 9,
          parentEventId,
          extraTags: tags,
          createdAt: Math.floor(Date.now() / 1000),
        }),
      input,
    );
    if (!event)
      throw new Error("The Outreach discussion fixture was not emitted.");
    return event;
  };

  await emit({
    content: "WhatsApp number connected.",
    tags: [["wa-notice", "connected"]],
  });
  await emit({
    content: "Batch sent. 12 guesthouses messaged, 9 read.",
    tags: [["wa-notice", "batch_completed"]],
  });
  await emit({
    content: "Scout prepared a batch. It is waiting for approval.",
    tags: [["wa-notice", "batch_pending"]],
  });
  const root = await emit({
    content: "WhatsApp: Plane Tree Guesthouse (+27 82 123 7135)",
    tags: [
      ["wa", "27821237135"],
      ["wa-name", "Plane Tree Guesthouse"],
    ],
  });
  await emit({
    content: "This looks great. What would it cost?",
    parentEventId: root.id,
    tags: [
      ["wa", "27821237135"],
      ["wa-name", "Plane Tree Guesthouse"],
      ["wa-dir", "in"],
    ],
  });
  await emit({
    content: "How did the Melville batch do?",
    pubkey: "deadbeef".repeat(8),
  });
  await emit({
    content: "Nine read and one replied. I will review the reply now.",
    pubkey: "c0ffee".repeat(8),
  });
}

async function openDesk(
  page: Page,
  theme: WhatsAppProofTheme,
  businessName = "Horizon Labs",
) {
  await installWhatsAppMockBridge(page, theme, {
    businessName,
    deskFixture: true,
  });
  await installLogoPreview(page);
  if ((page.viewportSize()?.width ?? 1440) < 600) {
    await gotoWhatsAppScreen(page, `/channels/${OUTREACH_CHANNEL_ID}`);
    await expect(page.getByTestId("chat-title")).toHaveText("Outreach");
    await expect(page.getByTestId("outreach-desk-needs-you")).toBeVisible();
    return;
  }
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await expect(page.getByTestId("home-inbox")).toBeVisible();
  await page.getByTestId("channel-Outreach").click();
  await expect(page.getByTestId("outreach-desk-needs-you")).toBeVisible();
}

async function capture(
  page: Page,
  fileName: string,
  width: number,
  proofDirectory: string,
) {
  await page.evaluate(() => {
    window.scrollTo(0, 0);
    document.querySelectorAll<HTMLElement>("body *").forEach((element) => {
      if (element.scrollTop !== 0 || element.scrollLeft !== 0) {
        element.scrollTop = 0;
        element.scrollLeft = 0;
      }
    });
  });
  await waitForAnimations(page);
  await page.screenshot({
    path: path.join(proofDirectory, fileName),
    fullPage: width === 1440,
    animations: "disabled",
  });
}

test("captures every Outreach desk tab at phone and desktop sizes in both themes", async ({
  browser,
}) => {
  test.setTimeout(300_000);
  const proofDirectory = process.env.WA_DESK_PROOF_DIR
    ? path.resolve(process.env.WA_DESK_PROOF_DIR)
    : test.info().outputPath("screenshots");
  await mkdir(proofDirectory, { recursive: true });

  for (const theme of ["light", "dark"] as const) {
    const context = await browser.newContext();
    const page = await context.newPage();
    try {
      for (const viewport of [
        { label: "390", width: 390, height: 844 },
        { label: "1440", width: 1440, height: 1600 },
      ]) {
        await page.setViewportSize(viewport);
        await openDesk(page, theme, "Northbank Studio");
        await expect(page.locator(".c3wa-desk-page-head")).toBeVisible();
        await expect(page.locator(".c3wa-desk-page-head")).toBeInViewport();
        await expect(
          page.getByRole("heading", { name: /things need you/u }),
        ).toBeVisible();
        await expect(page.getByTestId("channel-view-tab-needs-you")).toHaveCSS(
          "white-space",
          "nowrap",
        );
        const suffix = `${viewport.label}-${theme}`;

        await capture(
          page,
          `needs-you-${suffix}.png`,
          viewport.width,
          proofDirectory,
        );

        await page.getByTestId("channel-view-tab-pipeline").click();
        await expect(page.getByTestId("outreach-desk-pipeline")).toBeVisible();
        await expect(
          page.getByRole("heading", { name: "Pipeline", exact: true }),
        ).toBeVisible();
        await expect(page.locator(".c3wa-desk-page-head")).toBeInViewport();
        await expect(page.getByPlaceholder("Find a business")).toBeVisible();
        await capture(
          page,
          `pipeline-${suffix}.png`,
          viewport.width,
          proofDirectory,
        );

        await page.getByTestId("channel-view-tab-chats").click();
        await expect(page.getByTestId("whatsapp-outreach")).toBeVisible();
        await capture(
          page,
          `chats-${suffix}.png`,
          viewport.width,
          proofDirectory,
        );
        if (viewport.width === 1440) {
          await page.getByTestId("whatsapp-contact-27821237135").click();
          await expect(
            page.getByTestId("whatsapp-reply-input"),
          ).toHaveAttribute(
            "placeholder",
            "Reply on WhatsApp as Northbank Studio",
          );
          await capture(
            page,
            `chats-conversation-${suffix}.png`,
            viewport.width,
            proofDirectory,
          );
        }
        if (viewport.width === 390) {
          await page.getByTestId("whatsapp-contact-27821237135").click();
          await expect(
            page.getByTestId("whatsapp-reply-input"),
          ).toHaveAttribute("placeholder", "Reply on WhatsApp…");
          const openRecord = page.getByRole("button", {
            name: "Record",
            exact: true,
          });
          await expect(openRecord).toBeVisible({ timeout: 10_000 });
          await capture(
            page,
            `chats-conversation-${suffix}.png`,
            viewport.width,
            proofDirectory,
          );
          await openRecord.focus({ timeout: 5_000 });
          await page.keyboard.press("Enter");
          await expect(page.getByTestId("whatsapp-contact-record")).toBeVisible(
            { timeout: 10_000 },
          );
          await capture(
            page,
            `chats-record-${suffix}.png`,
            viewport.width,
            proofDirectory,
          );
        }

        await page.getByTestId("channel-view-tab-discussion").click();
        await emitDiscussionFixture(page);
        await expect(
          page.getByTestId("whatsapp-discussion-activity"),
        ).toBeVisible();
        const scrollToLatest = page.getByTestId("thread-scroll-to-latest");
        if (await scrollToLatest.isVisible()) {
          await scrollToLatest.click();
          await expect(scrollToLatest).toHaveCount(0);
        }
        await capture(
          page,
          `discussion-${suffix}.png`,
          viewport.width,
          proofDirectory,
        );

        await page.getByTestId("channel-view-tab-knowledge").click();
        await expect(page.getByTestId("work-area-panel")).toBeVisible();
        await expect(page.getByTestId("work-area-knowledge")).toBeVisible();
        await capture(
          page,
          `knowledge-${suffix}.png`,
          viewport.width,
          proofDirectory,
        );
        await page.getByTestId("work-area-close").click();

        await page.getByTestId("channel-view-tab-canvas").click();
        await expect(page.getByTestId("work-area-panel")).toBeVisible();
        await expect(page.getByTestId("work-area-canvas")).toBeVisible();
        await capture(
          page,
          `canvas-${suffix}.png`,
          viewport.width,
          proofDirectory,
        );
        await page.getByTestId("work-area-close").click();

        await page.getByTestId("channel-view-tab-files").click();
        await expect(page.getByTestId("work-area-panel")).toBeVisible();
        await expect(
          page.locator("[data-testid^='work-area-files']").first(),
        ).toBeVisible();
        await capture(
          page,
          `files-${suffix}.png`,
          viewport.width,
          proofDirectory,
        );
        await page.getByTestId("work-area-close").click();
      }
    } finally {
      await context.close();
    }
  }
});

test("captures the C11 connect state with the active business name", async ({
  browser,
}) => {
  test.setTimeout(120_000);
  const proofDirectory = process.env.WA_DESK_PROOF_DIR
    ? path.resolve(process.env.WA_DESK_PROOF_DIR)
    : test.info().outputPath("screenshots");
  await mkdir(proofDirectory, { recursive: true });

  for (const theme of ["light", "dark"] as const) {
    for (const viewport of [
      { label: "390", width: 390, height: 844 },
      { label: "1440", width: 1440, height: 900 },
    ]) {
      const context = await browser.newContext({ viewport });
      try {
        const page = await context.newPage();
        await installWhatsAppMockBridge(page, theme, {
          businessName: "Northbank Studio",
          connected: false,
        });
        await gotoWhatsAppScreen(
          page,
          `/channels/${OUTREACH_CHANNEL_ID}`,
          "whatsapp=disconnected",
        );
        await expect
          .poll(() =>
            page.evaluate(
              () =>
                (
                  window as Window & {
                    __BUZZ_E2E_WHATSAPP_STATE__?: {
                      status?: { connected?: boolean };
                    };
                  }
                ).__BUZZ_E2E_WHATSAPP_STATE__?.status?.connected,
            ),
          )
          .toBe(false);
        await page.getByTestId("channel-view-tab-chats").click();
        const connectState = page.locator(".c3wa-outreach-empty");
        await expect(
          connectState.getByRole("heading", {
            name: "WhatsApp is not connected",
          }),
        ).toBeVisible();
        await expect(connectState).toContainText(
          "Connect your Northbank Studio number in Settings to open Outreach.",
        );
        await expect(connectState).not.toContainText("Horizon Labs");
        await waitForAnimations(page);
        await page.screenshot({
          path: path.join(
            proofDirectory,
            `c11-connect-${viewport.label}-${theme}.png`,
          ),
          fullPage: true,
          animations: "disabled",
        });
      } finally {
        await context.close();
      }
    }
  }
});
