import { expect, test, type Page } from "@playwright/test";

import {
  CORNER_COTTAGE_WA_ID,
  GREEN_SHUTTER_WA_ID,
  JACARANDA_WA_ID,
  OUTREACH_CHANNEL_ID,
  installWhatsAppMockBridge,
  gotoWhatsAppScreen,
} from "../helpers/whatsappOutreach";

const PLANE_TREE_WA_ID = "27821237135";
const UNLINKED_WA_ID = "27683735905";
const NO_WEBSITE_WA_ID = "27795552241";

type WhatsAppMockState = {
  commands: Array<{ kind: number; body: Record<string, unknown> }>;
  contacts: Array<{
    waId: string;
    stage: string;
    label: string | null;
    displayName: string | null;
  }>;
};

type WhatsAppEventWindow = Window & {
  __BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
    channelName: string;
  }) => boolean;
  __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
    channelName: string;
    content: string;
    pubkey: string;
    kind: number;
    extraTags?: string[][];
    createdAt?: number;
  }) => { id: string };
  __BUZZ_E2E_WHATSAPP_STATE__?: WhatsAppMockState;
  __BUZZ_E2E_WHATSAPP__?: (request: {
    action: "command";
    kind: number;
    body: Record<string, unknown>;
  }) => Promise<unknown>;
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

async function openOutreachFromHome(page: Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await expect(page.getByTestId("home-inbox")).toBeVisible();
  await page.getByTestId("channel-Outreach").click();
  await expect(page.getByTestId("chat-title")).toHaveText("Outreach");
  await expect(page.getByTestId("outreach-desk-needs-you")).toBeVisible();
}

async function emitOutreachEvent(
  page: Page,
  input: { content: string; tags?: string[][]; pubkey?: string },
) {
  await expect
    .poll(() =>
      page.evaluate(
        (channelName) =>
          (
            window as WhatsAppEventWindow
          ).__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({ channelName }) ?? false,
        "Outreach",
      ),
    )
    .toBe(true);
  const event = await page.evaluate(
    ({ content, tags, pubkey }) =>
      (window as WhatsAppEventWindow).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "Outreach",
        content,
        pubkey: pubkey ?? "deadbeef".repeat(8),
        kind: 9,
        extraTags: tags,
        createdAt: Math.floor(Date.now() / 1000),
      }),
    input,
  );
  if (!event) throw new Error("The mock Outreach message was not emitted.");
  return event;
}

test.describe("Outreach desk", () => {
  test("chat and pipeline use the active business name", async ({ page }) => {
    const businessName = "Northbank Studio";
    await installWhatsAppMockBridge(page, "light", {
      businessName,
      deskFixture: true,
    });
    await page.setViewportSize({ width: 1440, height: 900 });
    await openOutreachFromHome(page);
    await expect(
      page.getByText(`Private WhatsApp conversations for ${businessName}.`),
    ).toBeVisible();
    await page.getByTestId("channel-view-tab-chats").click();
    await page.getByTestId(`whatsapp-contact-${PLANE_TREE_WA_ID}`).click();
    await expect
      .soft(page.getByTestId("whatsapp-reply-input"))
      .toHaveAttribute("placeholder", `Reply on WhatsApp as ${businessName}`);
    await page.getByTestId("channel-view-tab-pipeline").click();
    await expect
      .soft(page.locator(".c3wa-desk-eyebrow").filter({ hasText: "WhatsApp" }))
      .toHaveText(`${businessName} · WhatsApp`);
    await expect
      .soft(page.getByTestId("outreach-desk-pipeline"))
      .not.toContainText("Horizon Labs");
  });

  test("opens on Needs you, approves a batch, books a call, and moves a pipeline card with the keyboard", async ({
    page,
  }) => {
    await installWhatsAppMockBridge(page, "light", { deskFixture: true });
    await installLogoPreview(page);
    await page.setViewportSize({ width: 1440, height: 900 });
    await openOutreachFromHome(page);

    const needs = page.getByTestId("outreach-desk-needs-you");
    const waitingReply = needs.getByTestId(
      `outreach-needs-contact-${PLANE_TREE_WA_ID}`,
    );
    await expect(waitingReply).toHaveCount(1);
    await expect(waitingReply).toContainText("Replied");
    await expect(
      waitingReply.locator("img[alt='Plane Tree Guesthouse']"),
    ).toHaveAttribute("src", /img\.logo\.dev\/form-field\.example/u);

    const wantsCall = needs.getByTestId(
      `outreach-needs-contact-${CORNER_COTTAGE_WA_ID}`,
    );
    await expect(wantsCall).toHaveCount(1);
    await expect(wantsCall).toContainText("Wants a call");

    const batch = needs.getByTestId("outreach-needs-batch-e2e-whatsapp-batch");
    await expect(
      batch.getByRole("button", { name: "Review each message" }),
    ).toBeVisible();
    await batch.getByRole("button", { name: /Approve and send/u }).click();
    await expect(batch).toHaveCount(0);

    const bookCall = wantsCall.getByRole("button", {
      name: "Mark call booked",
    });
    await expect(bookCall).toBeEnabled({ timeout: 15_000 });
    await bookCall.click();
    await expect
      .poll(() =>
        page.evaluate(
          (waId) =>
            (
              window as WhatsAppEventWindow
            ).__BUZZ_E2E_WHATSAPP_STATE__?.commands.some(
              ({ kind, body }) =>
                kind === 47052 &&
                body.waId === waId &&
                body.stage === "call_booked",
            ) ?? false,
          CORNER_COTTAGE_WA_ID,
        ),
      )
      .toBe(true);

    await page.getByTestId("channel-view-tab-pipeline").click();
    const callBookedColumn = page.locator(
      '.c3wa-pipeline-column[data-stage="call_booked"]',
    );
    await expect(
      callBookedColumn.getByTestId(
        `outreach-pipeline-card-${CORNER_COTTAGE_WA_ID}`,
      ),
    ).toBeVisible();
    await expect(
      page
        .getByRole("region", { name: "Contacted" })
        .getByTestId(`outreach-pipeline-card-${JACARANDA_WA_ID}`),
    ).toContainText("Waiting for a reply, read");
    await expect(
      page
        .getByRole("region", { name: "Contacted" })
        .getByTestId(`outreach-pipeline-card-${GREEN_SHUTTER_WA_ID}`),
    ).not.toContainText("Waiting for a reply");

    const moveButton = page
      .getByRole("region", { name: "WhatsApp pipeline stages" })
      .getByTestId(`outreach-move-${JACARANDA_WA_ID}`);
    await moveButton.focus();
    await expect(moveButton).toBeFocused();
    await page.keyboard.press("Enter");
    const quoteSentOption = page.getByRole("menuitem", {
      name: "Quote sent",
      exact: true,
    });
    await expect(quoteSentOption).toBeVisible();
    await page.keyboard.press("q");
    await expect(quoteSentOption).toHaveAttribute("data-highlighted", "");
    await page.keyboard.press("Enter");
    await expect
      .poll(() =>
        page.evaluate(
          (waId) =>
            (
              window as WhatsAppEventWindow
            ).__BUZZ_E2E_WHATSAPP_STATE__?.commands.some(
              ({ kind, body }) =>
                kind === 47052 &&
                body.waId === waId &&
                body.stage === "quote_sent",
            ) ?? false,
          JACARANDA_WA_ID,
        ),
      )
      .toBe(true);
  });

  test("renames a contact across Chats and Pipeline and shows fallback identities", async ({
    page,
  }) => {
    await installWhatsAppMockBridge(page, "light", { deskFixture: true });
    await installLogoPreview(page);
    await page.setViewportSize({ width: 1440, height: 900 });
    await openOutreachFromHome(page);
    await page.getByTestId("channel-view-tab-chats").click();

    const unlinkedRow = page.getByTestId(`whatsapp-contact-${UNLINKED_WA_ID}`);
    await expect(unlinkedRow).toContainText("+27 68 373 5905");
    await expect(
      unlinkedRow.getByText("+27 68 373 5905", { exact: true }),
    ).toHaveCount(1);
    await expect(unlinkedRow.locator("svg.lucide-user-round")).toBeVisible();

    const linkedRow = page.getByTestId(`whatsapp-contact-${JACARANDA_WA_ID}`);
    await expect(linkedRow).toContainText("Mpho");
    await expect(linkedRow).not.toContainText("+27 82 123 4410");

    const noWebsiteRow = page.getByTestId(
      `whatsapp-contact-${NO_WEBSITE_WA_ID}`,
    );
    await expect(noWebsiteRow.locator(".business-logo-fallback")).toHaveText(
      "L",
    );
    await expect(noWebsiteRow.locator("img")).toHaveCount(0);
    await page.getByTestId(`whatsapp-contact-${JACARANDA_WA_ID}`).click();
    await page.getByTestId("whatsapp-rename-contact").click();
    await page.getByLabel("Contact label").fill("Jacaranda House");
    await page.getByRole("button", { name: "Save label" }).click();
    await expect
      .poll(() =>
        page.evaluate(
          (waId) =>
            (
              window as WhatsAppEventWindow
            ).__BUZZ_E2E_WHATSAPP_STATE__?.commands.some(
              ({ kind, body }) =>
                kind === 47052 &&
                body.waId === waId &&
                body.label === "Jacaranda House",
            ) ?? false,
          JACARANDA_WA_ID,
        ),
      )
      .toBe(true);
    await expect(
      page.getByTestId(`whatsapp-contact-${JACARANDA_WA_ID}`),
    ).toContainText("Jacaranda House");

    await page.getByTestId("channel-view-tab-pipeline").click();
    await expect(
      page.getByTestId(`outreach-pipeline-card-${JACARANDA_WA_ID}`).first(),
    ).toContainText("Jacaranda House");
    await page.getByTestId("channel-view-tab-chats").click();
    await page.getByTestId(`whatsapp-contact-${JACARANDA_WA_ID}`).click();
    await page.getByTestId("whatsapp-rename-contact").click();
    await expect(page.getByLabel("Contact label")).toHaveAttribute(
      "maxlength",
      "120",
    );
    await page.getByRole("button", { name: "Clear label" }).click();
    await expect
      .poll(() =>
        page.evaluate(
          (waId) =>
            (
              window as WhatsAppEventWindow
            ).__BUZZ_E2E_WHATSAPP_STATE__?.contacts.find(
              (contact) => contact.waId === waId,
            )?.displayName,
          JACARANDA_WA_ID,
        ),
      )
      .toBe("Mpho");
  });

  test("plain members have no stage or rename controls", async ({ page }) => {
    await installWhatsAppMockBridge(page, "light", {
      deskFixture: true,
      outreachChannelMember: false,
      relayRole: "member",
    });
    await page.setViewportSize({ width: 1440, height: 900 });
    await gotoWhatsAppScreen(page, `/channels/${OUTREACH_CHANNEL_ID}`);
    await expect(page.getByTestId("chat-title")).toHaveText("Outreach");
    await expect(page.getByTestId("outreach-desk-needs-you")).toBeVisible();
    await page.getByTestId("channel-view-tab-pipeline").click();
    await expect(
      page.getByTestId(`outreach-move-${JACARANDA_WA_ID}`),
    ).toHaveCount(0);
    await page.getByTestId("channel-view-tab-chats").click();
    await page.getByTestId(`whatsapp-contact-${JACARANDA_WA_ID}`).click();
    await expect(page.getByTestId("whatsapp-rename-contact")).toHaveCount(0);
    const refusal = await page.evaluate(async (waId) => {
      const event = window as WhatsAppEventWindow;
      return event.__BUZZ_E2E_WHATSAPP__?.({
        action: "command",
        kind: 47052,
        body: { waId, stage: "quote_sent" },
      });
    }, JACARANDA_WA_ID);
    await expect
      .poll(() =>
        page.evaluate(
          (waId) =>
            (
              window as WhatsAppEventWindow
            ).__BUZZ_E2E_WHATSAPP_STATE__?.contacts.find(
              (contact) => contact.waId === waId,
            )?.stage,
          JACARANDA_WA_ID,
        ),
      )
      .toBe("contacted");
    expect(refusal).toEqual({
      message:
        "only the owner, admins, their agents and Outreach members can change a contact's stage or name",
    });
  });

  test("Discussion folds WhatsApp activity and removes pending batch notices", async ({
    page,
  }) => {
    await installWhatsAppMockBridge(page, "light", { deskFixture: true });
    await page.setViewportSize({ width: 1440, height: 900 });
    await openOutreachFromHome(page);
    const root = await emitOutreachEvent(page, {
      content: "WhatsApp number connected.",
      tags: [["wa-notice", "connected"]],
    });
    await emitOutreachEvent(page, {
      content: "A batch is waiting for approval.",
      tags: [
        ["wa-notice", "batch_pending"],
        ["wa-batch", "e2e-whatsapp-batch"],
      ],
    });
    await emitOutreachEvent(page, {
      content: "Batch sent. Three guesthouses messaged.",
      tags: [
        ["wa-notice", "batch_completed"],
        ["wa-batch", "e2e-whatsapp-batch"],
      ],
    });
    await emitOutreachEvent(page, {
      content: "WhatsApp: Corner Cottage Parkhurst (+27 72 555 1188)",
      tags: [
        ["wa", CORNER_COTTAGE_WA_ID],
        ["wa-name", "Corner Cottage Parkhurst"],
      ],
    });
    await emitOutreachEvent(page, { content: "A normal team note." });

    await page.getByTestId("channel-view-tab-discussion").click();
    const activity = page.getByTestId("whatsapp-discussion-activity");
    await expect(activity).toContainText("3 updates");
    await expect(activity).toContainText("Number connected");
    await expect(activity).toContainText("Batch sent");
    await expect(activity).toContainText("Corner Cottage Parkhurst");
    await expect(page.getByText("A normal team note.")).toBeVisible();
    await expect(
      page.getByText("A batch is waiting for approval."),
    ).toHaveCount(0);
    await activity.locator("summary").click();
    await expect(
      activity.getByRole("button", { name: "Open chat", exact: true }),
    ).toBeVisible();
    expect(root.id).toBeTruthy();
  });
});
