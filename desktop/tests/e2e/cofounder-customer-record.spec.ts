import { mkdirSync } from "node:fs";
import { resolve } from "node:path";

import { expect, test } from "@playwright/test";
import { bytesToHex } from "@noble/hashes/utils.js";
import { generateSecretKey, getPublicKey } from "nostr-tools/pure";

import { TEST_IDENTITIES, installMockBridge } from "../helpers/bridge";
import {
  cedarCafeLeadRecord,
  planeTreeCustomerRecord,
} from "../../src/testing/cofounderCustomerFixtures";
import {
  REFERENCE_CHANNEL_IDS,
  REFERENCE_PROSPECT_IDS,
} from "../../src/testing/e2eReferenceWorkspace";
import type {
  MockCompanyFunctionHead,
  MockManagedAgentSeed,
} from "../../src/testing/e2eBridge";
import { waitForAnimations } from "../helpers/animations";

const OWNER_PUBKEY = "deadbeef".repeat(8);
const OLIVE_PARTY_ID = "a1a17000-0000-4000-8000-000000000001";
const MELVILLE_PARTY_ID = REFERENCE_PROSPECT_IDS["the-olive-house"];
const PLANE_TREE_PARTY_ID = REFERENCE_PROSPECT_IDS["form-field"];
const EMPTY_PARTY_ID = "5b2e2e1d-3308-4e4f-958e-9509070626d1";

function customerFixture({
  enabled = true,
  role = "owner",
  whatsappGuesthouses = true,
  companyFunctionHeads,
  managedAgents,
  customerActionErrors,
  manualMelville = false,
}: {
  enabled?: boolean;
  role?: "owner" | "admin" | "member";
  whatsappGuesthouses?: boolean;
  companyFunctionHeads?: MockCompanyFunctionHead[];
  managedAgents?: MockManagedAgentSeed[];
  customerActionErrors?: string[];
  manualMelville?: boolean;
} = {}) {
  const relaySecret = generateSecretKey();
  const input = {
    scoutPubkey: TEST_IDENTITIES.alice.pubkey,
    ayaPubkey: TEST_IDENTITIES.bob.pubkey,
    ownerPubkey: OWNER_PUBKEY,
  };
  const planeTree = planeTreeCustomerRecord(input);
  planeTree.partyId = PLANE_TREE_PARTY_ID;
  planeTree.leadId = PLANE_TREE_PARTY_ID;

  const melville = cedarCafeLeadRecord(input);
  melville.partyId = MELVILLE_PARTY_ID;
  melville.leadId = MELVILLE_PARTY_ID;
  melville.displayName = "Melville Manor Guesthouse";
  melville.found = {
    at: 1_793_000_000,
    byPubkey: input.ayaPubkey,
    search: "guesthouses in Melville",
  };
  if (manualMelville) {
    melville.found = null;
    melville.timeline = (melville.timeline as Array<{ kind: string }>).filter(
      (entry) => entry.kind !== "found",
    );
  }

  const olive = structuredClone(planeTree);
  olive.partyId = OLIVE_PARTY_ID;
  olive.displayName = "The Olive House";
  olive.clientId = REFERENCE_CHANNEL_IDS.oliveHouse;
  olive.stage = "client";

  return {
    relaySelf: getPublicKey(relaySecret),
    companyMemberRelayPrivateKeyHex: bytesToHex(relaySecret),
    referenceWorkspace: true,
    referenceWorkspaceRole: role,
    whatsappGuesthouses,
    relayRole: role,
    relayRequiresMembership: true,
    relayMembers: [{ pubkey: OWNER_PUBKEY, role }],
    cofounderSettings: {
      enabled,
      moneyPreference: "cheapest_first" as const,
      approvalSwitches: {
        approveEveryHire: false,
        approvePlaybookChanges: false,
        approveEveryDiscoveryCampaign: false,
      },
    },
    companyFunctionHeads,
    managedAgents,
    companyCustomerRecords: [planeTree, melville, olive],
    ...(customerActionErrors ? { customerActionErrors } : {}),
  };
}

async function openCustomerRecord(
  page: import("@playwright/test").Page,
  partyId: string,
  fixture = customerFixture(),
  theme?: "light" | "dark",
) {
  await installMockBridge(page, fixture);
  if (theme) {
    await page.emulateMedia({ colorScheme: theme });
    await page.addInitScript(
      ({ selectedTheme, ownerPubkey }) => {
        const themeName = selectedTheme === "dark" ? "buzz-dark" : "buzz";
        for (const key of Object.keys(window.localStorage)) {
          if (key.startsWith("colony.appearance.v1:")) {
            window.localStorage.removeItem(key);
          }
        }
        window.localStorage.removeItem("buzz-theme-cache");
        window.localStorage.setItem("buzz-theme", themeName);
        window.localStorage.setItem("buzz-follow-system", "false");
        window.localStorage.setItem("buzz-accent-color", "#5B45BE");
        window.localStorage.setItem(
          `buzz-community-theme.v1:${ownerPubkey}:${encodeURIComponent("ws://localhost:3000")}`,
          JSON.stringify({
            version: 1,
            theme: themeName,
            accent: "#5B45BE",
            followSystem: false,
          }),
        );
      },
      { selectedTheme: theme, ownerPubkey: OWNER_PUBKEY },
    );
  }
  await page.addInitScript((outreachChannelId) => {
    const testWindow = window as Window & {
      __BUZZ_E2E_WHATSAPP__?: (request: {
        action: "post" | "command";
        path?: string;
        body?: Record<string, unknown>;
      }) => Promise<unknown>;
    };
    testWindow.__BUZZ_E2E_WHATSAPP__ = async (request) => {
      if (
        request.action === "post" &&
        request.path === "/api/whatsapp/query" &&
        request.body?.what === "status"
      ) {
        return {
          connected: true,
          connectionId: "cof4-whatsapp-connection",
          displayPhoneNumber: "+27 61 534 0627",
          verifiedName: "Horizon Labs",
          quality: "GREEN",
          phoneStatus: "CONNECTED",
          dailyNewChatCap: 30,
          newChatsSentToday: 0,
          sendingPaused: false,
          pauseReason: null,
          issues: [],
          outreachChannelId,
          templatesSyncedAt: null,
          templates: [],
          webhookUrl: null,
        };
      }
      return {};
    };
  }, REFERENCE_CHANNEL_IDS.sales);
  await page.goto(`/#/clients/record/${partyId}`);
  await expect(page.getByTestId("customer-record-screen")).toBeVisible();
}

test.describe("co-founder customer records", () => {
  test("owner opens a client record from Today and returns to Clients", async ({
    page,
  }) => {
    await installMockBridge(
      page,
      customerFixture({ whatsappGuesthouses: false }),
    );
    await page.goto("/#/today");

    await page.getByTestId("sidebar-nav-business").click();
    await page.getByTestId("sidebar-business-clients").click();
    await expect(page.getByRole("table", { name: "Clients" })).toContainText(
      "The Olive House",
    );
    await page.getByRole("link", { name: "The Olive House" }).click();

    await expect(page).toHaveURL(
      new RegExp(`/clients/record/${OLIVE_PARTY_ID}$`),
    );
    await expect(
      page.getByRole("heading", { name: "The Olive House", exact: true }),
    ).toBeVisible();
    await page
      .getByTestId("app-top-chrome")
      .getByRole("button", { name: "Clients", exact: true })
      .click();
    await expect(page).toHaveURL(/\/clients$/);
  });

  test("Make client keeps every lead timeline entry", async ({ page }) => {
    await installMockBridge(page, customerFixture());
    await page.goto("/#/leads");

    const row = page
      .getByRole("table", { name: "Leads" })
      .getByRole("row")
      .filter({ hasText: "Melville Manor Guesthouse" });
    await row.getByRole("link", { name: "Review lead" }).click();
    await expect(page).toHaveURL(
      new RegExp(`/sales/lead/${MELVILLE_PARTY_ID}$`),
    );
    const detail = page.getByTestId("w10-prospect-page");
    const convert = detail.getByTestId(`lead-make-client-${MELVILLE_PARTY_ID}`);
    await expect(convert).toBeVisible();
    await convert.click();
    await expect(convert).toHaveCount(0);
    await detail.getByRole("link", { name: "Customer record" }).click();

    const timeline = page
      .getByTestId("customer-record-screen")
      .locator("ol li");
    await expect(timeline).toHaveCount(3);
    await expect(timeline.nth(0)).toContainText("Found in Discovery");
    await expect(timeline.nth(1)).toContainText("WhatsApp opener approved");
    await expect(timeline.nth(2)).toContainText("Became a client");
  });

  test("a manually added lead keeps truthful history and appears as one client after conversion", async ({
    page,
  }) => {
    await installMockBridge(page, customerFixture({ manualMelville: true }));
    await page.goto("/#/leads");

    const leads = page.getByRole("table", { name: "Leads" });
    const lead = leads.getByRole("row").filter({
      hasText: "Melville Manor Guesthouse",
    });
    const reviewLeadLink = lead.getByRole("link", {
      name: "Review lead",
    });
    await expect(reviewLeadLink).toBeVisible();
    await reviewLeadLink.click();
    await expect(page).toHaveURL(
      new RegExp(`/sales/lead/${MELVILLE_PARTY_ID}$`),
    );

    const customerRecordLink = page
      .getByTestId("w10-prospect-page")
      .getByRole("link", {
        name: "Customer record",
      });
    await expect(customerRecordLink).toBeVisible();
    await customerRecordLink.click();
    await expect(page).toHaveURL(
      new RegExp(`/clients/record/${MELVILLE_PARTY_ID}$`),
    );
    const timeline = page
      .getByTestId("customer-record-screen")
      .locator("ol li");
    await expect(timeline).not.toContainText("Found in Discovery");

    await page.goto("/#/leads");
    const leadToConvert = page
      .getByRole("table", { name: "Leads" })
      .getByRole("row")
      .filter({ hasText: "Melville Manor Guesthouse" });
    await leadToConvert.getByRole("link", { name: "Review lead" }).click();
    const leadDetail = page.getByTestId("w10-prospect-page");
    const convert = leadDetail.getByTestId(
      `lead-make-client-${MELVILLE_PARTY_ID}`,
    );
    await convert.click();
    await expect(convert).toHaveCount(0);

    await page.goto("/#/leads");
    const convertedLead = page
      .getByRole("table", { name: "Leads" })
      .getByRole("row")
      .filter({ hasText: "Melville Manor Guesthouse" });
    await expect.soft(convertedLead).toContainText("Client");

    await page.goto("/#/clients");
    const client = page.getByTestId(`client-row-${MELVILLE_PARTY_ID}`);
    await expect.soft(client).toContainText("Melville Manor Guesthouse");
    await expect
      .soft(page.getByRole("table", { name: "Clients" }).locator("tbody tr"))
      .toHaveCount(3);
    if ((await client.count()) > 0) {
      await client
        .getByRole("link", { name: "Melville Manor Guesthouse" })
        .click();
      await expect(page).toHaveURL(
        new RegExp(`/clients/record/${MELVILLE_PARTY_ID}$`),
      );
      await expect(
        page
          .getByTestId("customer-record-screen")
          .locator("ol li")
          .filter({ hasText: "Became a client" }),
      ).toContainText("Became a client");
    }
  });

  test("a Sales lead can convert a lead while still a plain member", async ({
    page,
  }) => {
    await installMockBridge(
      page,
      customerFixture({
        role: "member",
        companyFunctionHeads: [
          {
            function: "sales",
            state: "filled",
            leadPubkey: OWNER_PUBKEY,
          },
        ],
      }),
    );
    await page.goto("/#/leads");

    const row = page
      .getByRole("table", { name: "Leads" })
      .getByRole("row")
      .filter({ hasText: "Melville Manor Guesthouse" });
    await row.getByRole("link", { name: "Review lead" }).click();
    await expect(page).toHaveURL(
      new RegExp(`/sales/lead/${MELVILLE_PARTY_ID}$`),
    );
    const detail = page.getByTestId("w10-prospect-page");
    const convert = detail.getByTestId(`lead-make-client-${MELVILLE_PARTY_ID}`);
    await expect(convert).toBeVisible();
    await convert.click();
    await expect(convert).toHaveCount(0);
    await detail.getByRole("link", { name: "Customer record" }).click();
    await expect(
      page.getByTestId("customer-record-screen").locator("ol li"),
    ).toHaveCount(3);
  });

  test("unknown money is shown as a dash", async ({ page }) => {
    await openCustomerRecord(page, MELVILLE_PARTY_ID);
    const money = page.locator('[aria-labelledby="customer-money-title"]');
    await expect(money.getByRole("heading", { name: "Money" })).toBeVisible();
    await expect(money.locator("dd")).toHaveText(["-", "-", "-"]);
  });

  test("a plain member sees the record without write actions", async ({
    page,
  }) => {
    await openCustomerRecord(
      page,
      MELVILLE_PARTY_ID,
      customerFixture({ role: "member" }),
    );

    await expect(
      page.getByRole("heading", { name: "Melville Manor Guesthouse" }),
    ).toBeVisible();
    await expect(page.getByRole("button", { name: "Make client" })).toHaveCount(
      0,
    );
    await expect(
      page.getByRole("button", { name: "Record milestone" }),
    ).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Attach file" })).toHaveCount(
      0,
    );
  });

  test("a Customer Success lead can edit but cannot convert a lead", async ({
    page,
  }) => {
    await openCustomerRecord(
      page,
      MELVILLE_PARTY_ID,
      customerFixture({
        role: "member",
        companyFunctionHeads: [
          {
            function: "customer_success",
            state: "filled",
            leadPubkey: OWNER_PUBKEY,
          },
        ],
      }),
    );

    await expect(
      page.getByRole("button", { name: "Record milestone" }),
    ).toBeVisible();
    await expect(page.getByRole("button", { name: "Make client" })).toHaveCount(
      0,
    );
    const timeline = page
      .getByTestId("customer-record-screen")
      .locator("ol li");
    await expect(timeline).toHaveCount(2);
    await page.getByRole("button", { name: "Record milestone" }).click();
    await page.getByRole("button", { name: "Save milestone" }).click();
    await expect(timeline).toHaveCount(3);
  });

  test("Scout can edit a record without conversion authority", async ({
    page,
  }) => {
    await openCustomerRecord(
      page,
      MELVILLE_PARTY_ID,
      customerFixture({
        role: "member",
        managedAgents: [
          {
            pubkey: OWNER_PUBKEY,
            name: "Scout",
            personaId: "builtin:fizz",
          },
        ],
      }),
    );

    await expect(
      page.getByRole("button", { name: "Record milestone" }),
    ).toBeVisible();
    await expect(page.getByRole("button", { name: "Make client" })).toHaveCount(
      0,
    );
    const timeline = page
      .getByTestId("customer-record-screen")
      .locator("ol li");
    await expect(timeline).toHaveCount(2);
    await page.getByRole("button", { name: "Record milestone" }).click();
    await page.getByRole("button", { name: "Save milestone" }).click();
    await expect(timeline).toHaveCount(3);
  });

  test("co-founder customer entry points stay hidden when mode is off", async ({
    page,
  }) => {
    await installMockBridge(page, customerFixture({ enabled: false }));
    await page.goto("/#/leads");
    await expect(
      page.getByRole("link", { name: "Customer record" }),
    ).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Make client" })).toHaveCount(
      0,
    );

    await page.goto(`/#/clients/record/${PLANE_TREE_PARTY_ID}`);
    await expect(page).toHaveURL(/\/clients$/);
    await expect(page.getByTestId("customer-record-screen")).toHaveCount(0);
  });

  test("missing customer records have an empty state", async ({ page }) => {
    await installMockBridge(page, customerFixture());
    await page.goto(`/#/clients/record/${EMPTY_PARTY_ID}`);
    await expect(page.getByTestId("customer-record-empty")).toContainText(
      "No customer record yet",
    );
    await expect(
      page.getByRole("button", { name: "Create record" }),
    ).toBeVisible();
  });

  test("stale head conflicts show the relay error and a retry", async ({
    page,
  }) => {
    const conflict = `conflict: customer record changed; current head is ${"c".repeat(64)}`;
    await openCustomerRecord(
      page,
      PLANE_TREE_PARTY_ID,
      customerFixture({ customerActionErrors: [conflict] }),
    );
    await page.getByRole("button", { name: "Record milestone" }).click();
    await page.getByRole("button", { name: "Save milestone" }).click();

    const alert = page.getByRole("alert");
    await expect(alert).toContainText(conflict);
    await alert.getByRole("button", { name: "Try again" }).click();
    await expect(page.getByRole("alert")).toHaveCount(0);
  });

  test("a refused file action is shown as a plain message", async ({
    page,
  }) => {
    const refusal = "restricted: only the record owner can attach this file";
    await openCustomerRecord(
      page,
      PLANE_TREE_PARTY_ID,
      customerFixture({ customerActionErrors: [refusal] }),
    );
    await page.getByRole("button", { name: "Attach file" }).first().click();
    await page.getByLabel("File name").fill("Launch assets");
    await page
      .getByLabel("Blossom file reference")
      .fill("blossom:sha256:launch-assets");
    await page.getByRole("button", { name: "Attach file" }).last().click();

    await expect(page.getByRole("alert")).toContainText(refusal);
  });

  test("an authorized owner can record a milestone and attach a file", async ({
    page,
  }) => {
    await openCustomerRecord(page, PLANE_TREE_PARTY_ID);
    const timeline = page
      .getByTestId("customer-record-screen")
      .locator("ol li");
    await expect(timeline).toHaveCount(18);

    await page.getByRole("button", { name: "Record milestone" }).click();
    await page.getByRole("button", { name: "Save milestone" }).click();
    await expect(timeline).toHaveCount(19);

    await page.getByRole("button", { name: "Attach file" }).first().click();
    await page.getByLabel("File name").fill("Launch assets");
    await page
      .getByLabel("Blossom file reference")
      .fill("blossom:sha256:launch-assets");
    await page.getByRole("button", { name: "Attach file" }).last().click();
    await expect(
      page.locator(".cf-customer-file-row").getByText("Launch assets"),
    ).toBeVisible();
  });

  test("Open chat selects the WhatsApp work area for this contact", async ({
    page,
  }) => {
    await openCustomerRecord(page, PLANE_TREE_PARTY_ID);
    await page.getByRole("button", { name: "Open chat" }).click();

    await expect(page).toHaveURL(/whatsappContactWaId=%2227825557135%22/);
    await expect(
      page.getByRole("tab", { name: "WhatsApp chats" }),
    ).toHaveAttribute("aria-selected", "true");
  });

  test("captures the record at desktop and phone widths", async ({ page }) => {
    test.skip(
      process.env.COF4_CAPTURE !== "1",
      "capture is requested for proof delivery",
    );
    const theme = process.env.COLONY3_PROOF_THEME === "dark" ? "dark" : "light";
    const output = resolve("/private/tmp/cof-ui-defects-proof/screenshots");
    mkdirSync(output, { recursive: true });
    await page.setViewportSize({ width: 1440, height: 960 });
    await openCustomerRecord(
      page,
      MELVILLE_PARTY_ID,
      customerFixture({ manualMelville: true }),
      theme,
    );
    await expect(
      page.getByRole("heading", { name: "Melville Manor Guesthouse" }),
    ).toBeVisible();
    await expect(
      page.getByTestId("customer-record-screen").locator("ol li"),
    ).not.toContainText("Found in Discovery");
    await waitForAnimations(page);
    await page.screenshot({
      path: resolve(output, `c9-manual-record-1440-${theme}.png`),
      fullPage: true,
    });

    await page.goto("/#/leads");
    const lead = page
      .getByRole("table", { name: "Leads" })
      .getByRole("row")
      .filter({ hasText: "Melville Manor Guesthouse" });
    await lead.getByTestId(`lead-make-client-${MELVILLE_PARTY_ID}`).click();
    await expect(lead).toContainText("Client");
    await page.getByRole("combobox", { name: "Stage" }).selectOption("client");
    await expect(lead).toBeVisible();
    await lead.scrollIntoViewIfNeeded();
    await waitForAnimations(page);
    await page.screenshot({
      path: resolve(output, `c9-leads-client-1440-${theme}.png`),
      fullPage: true,
    });

    await page.goto("/#/clients");
    const client = page.getByTestId(`client-row-${MELVILLE_PARTY_ID}`);
    await expect(client).toContainText("Melville Manor Guesthouse");
    await client.scrollIntoViewIfNeeded();
    await waitForAnimations(page);
    await page.screenshot({
      path: resolve(output, `c9-clients-row-1440-${theme}.png`),
      fullPage: true,
    });
    await client
      .getByRole("link", { name: "Melville Manor Guesthouse" })
      .click();
    await expect(page).toHaveURL(
      new RegExp(`/clients/record/${MELVILLE_PARTY_ID}$`),
    );
    await expect(
      page
        .getByTestId("customer-record-screen")
        .locator("ol li")
        .filter({ hasText: "Became a client" }),
    ).toBeVisible();
    await waitForAnimations(page);
    await page.screenshot({
      path: resolve(output, `c9-converted-record-1440-${theme}.png`),
      fullPage: true,
    });

    await page.setViewportSize({ width: 390, height: 844 });
    await waitForAnimations(page);
    await page.screenshot({
      path: resolve(output, `c9-converted-record-390-${theme}.png`),
      fullPage: true,
    });
    await page.goto("/#/leads");
    await page.getByRole("combobox", { name: "Stage" }).selectOption("client");
    await expect(
      page
        .getByRole("table", { name: "Leads" })
        .getByRole("row")
        .filter({ hasText: "Melville Manor Guesthouse" }),
    ).toContainText("Client");
    await waitForAnimations(page);
    await page.screenshot({
      path: resolve(output, `c9-leads-client-390-${theme}.png`),
      fullPage: true,
    });
    await page.goto("/#/clients");
    await expect(
      page.getByTestId(`client-row-${MELVILLE_PARTY_ID}`),
    ).toContainText("Melville Manor Guesthouse");
    await waitForAnimations(page);
    await page.screenshot({
      path: resolve(output, `c9-clients-row-390-${theme}.png`),
      fullPage: true,
    });
  });
});
