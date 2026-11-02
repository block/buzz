import { mkdir } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { expect, test, type Page } from "@playwright/test";
import { bytesToHex } from "@noble/hashes/utils.js";
import { generateSecretKey, getPublicKey } from "nostr-tools/pure";

import type { CofounderOnboardingStep } from "../../src/features/cofounder/cofounderPhase2Model";
import {
  ESIGN_PLAN_ID,
  esignSetupPlan,
  NOVEMBER_PLAN_ID,
  novemberPlan,
} from "../../src/testing/cofounderPlanFixtures";
import {
  REFERENCE_AGENTS,
  REFERENCE_CHANNEL_IDS,
} from "../../src/testing/e2eReferenceWorkspace";
import { KIND_WHATSAPP_BATCH_DECISION } from "../../src/shared/constants/kinds";
import type { MockCompanyAccessRecord } from "../../src/testing/mockCompanyAccess";
import { installMockBridge, type RelayBridgeIdentity } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";
import { installBrowserHostFake } from "./helpers/browserHostFake";
import { TEST_IDENTITIES } from "../helpers/bridge";
import type {
  MockCofounderLeadStatus,
  MockNumbers,
} from "../../src/testing/e2eBridge";

const APPROVAL_SWITCHES = {
  approveEveryHire: true,
  approvePlaybookChanges: true,
  approveEveryDiscoveryCampaign: true,
};

const NEEDS_YOU_NUMBERS: MockNumbers = {
  reviews: [
    {
      schemaVersion: 1,
      reviewDate: "2026-10-30",
      asOf: 1_791_273_600,
      windowStart: 1_790_668_800,
      previousStart: 1_790_064_000,
      currency: "ZAR",
      otherCurrencyFacts: 0,
      functions: [
        { function: "sales", metrics: [], notTracked: [] },
        { function: "operations", metrics: [], notTracked: [] },
        { function: "customer_success", metrics: [], notTracked: [] },
        { function: "finance", metrics: [], notTracked: [] },
        { function: "brand", metrics: [], notTracked: [] },
      ],
      wins: [],
      misses: [],
      decisions: [
        {
          decisionId: "hire-customer-success",
          title: "Hire a Customer Success lead?",
          detail:
            "Two sites go live next week. Someone should look after those clients so Aya can keep selling. Runs on your AI subscription.",
          function: "customer_success",
          state: "open",
          decidedByPubkey: null,
          decidedAt: null,
          note: null,
        },
      ],
      annotatedByPubkey: REFERENCE_AGENTS.scout.pubkey,
      annotatedAt: 1_791_273_600,
      createdAt: 1_791_273_600,
    },
  ],
};

const CAPABILITY_GAP_ID = "4f60a4ae-70c8-4f7a-a899-338e6b376198";
const PAYMENT_GAP_ID = "b986cc6f-57db-45a7-92b0-238d2855e881";
const NEEDS_YOU_WA_BATCH_ID = "cofounder-needs-you-batch";
const NEEDS_YOU_WA_DRAFT_ID = "cofounder-needs-you-draft";

function accessRecord(
  accessId: string,
  name: string,
  category: string,
  usedFor: string,
  cost: string,
): MockCompanyAccessRecord {
  const now = 1_791_273_600;
  return {
    dTag: `company:access:${accessId}`,
    content: {
      schemaVersion: 1,
      accessId,
      entry: {
        name,
        category,
        usedFor,
        agents: [REFERENCE_AGENTS.aya.pubkey],
        cost,
      },
      state: "active",
      grantedByPubkey: TEST_IDENTITIES.tyler.pubkey,
      grantedAt: now,
      viaGapId: null,
      revokedByPubkey: null,
      revokedAt: null,
      revokeReason: null,
      updatedAt: now,
    },
  };
}

function phase2AccessRecords(): MockCompanyAccessRecord[] {
  return [
    accessRecord(
      "esign",
      "E-sign, free plan",
      "esign",
      "Agreements to sign",
      "Free, 3 a month",
    ),
    accessRecord(
      "gmail",
      "Gmail",
      "connected_account",
      "Replying to customers",
      "Free",
    ),
    accessRecord(
      "google-calendar",
      "Google Calendar",
      "calendar",
      "Booking calls",
      "Free",
    ),
    accessRecord(
      "pinterest",
      "Pinterest",
      "connected_account",
      "Posting launch content",
      "Free",
    ),
    paymentLinkGapRecord(),
  ];
}

function capabilityGapRecord(): MockCompanyAccessRecord {
  const now = 1_791_273_600;
  return {
    dTag: `company:capgap:${CAPABILITY_GAP_ID}`,
    content: {
      schemaVersion: 1,
      gapId: CAPABILITY_GAP_ID,
      gap: {
        title: "Connect the e-sign account",
        need: "Scout needs a signing account to prepare customer agreements.",
        options: [
          {
            name: "Free e-sign plan",
            costText: "Free, 3 a month",
            costCents: 0,
            recurring: false,
            works: true,
            note: null,
          },
        ],
        chosen: 0,
        chosenReason: null,
        humanStep: "Sign in to the e-sign account",
        pageUrl: "https://accounts.example.test/esign/login",
        accessId: "esign",
        access: {
          name: "E-sign, free plan",
          category: "esign",
          usedFor: "Agreements to sign",
          agents: [REFERENCE_AGENTS.aya.pubkey],
          cost: "Free, 3 a month",
        },
      },
      state: "proposed",
      proposedByPubkey: REFERENCE_AGENTS.scout.pubkey,
      proposedAt: now,
      approvedCostCents: null,
      updatedAt: now,
    },
  };
}

function needsYouAccessRecords(): MockCompanyAccessRecord[] {
  const approvedGap = capabilityGapRecord();
  approvedGap.content = {
    ...approvedGap.content,
    state: "approved",
    approvedCostCents: 0,
  };
  return [...phase2AccessRecords(), approvedGap];
}

function paymentLinkGapRecord(): MockCompanyAccessRecord {
  const now = 1_791_273_600;
  return {
    dTag: `company:capgap:${PAYMENT_GAP_ID}`,
    content: {
      schemaVersion: 1,
      gapId: PAYMENT_GAP_ID,
      gap: {
        title: "Connect a payment link account",
        need: "To send deposit links from Colony and see when they are paid.",
        options: [
          {
            name: "Payment links",
            costText: "Free to join, a fee per payment",
            costCents: 0,
            recurring: false,
            works: true,
            note: null,
          },
        ],
        chosen: 0,
        chosenReason: null,
        humanStep: "Sign in to the payment link account",
        pageUrl: "https://accounts.example.test/payment-link/login",
        accessId: "payment-links",
        access: {
          name: "Payment link account",
          category: "payment_link",
          usedFor: "Sending deposit links",
          agents: [REFERENCE_AGENTS.scout.pubkey],
          cost: "Fee per payment",
        },
      },
      state: "proposed",
      proposedByPubkey: REFERENCE_AGENTS.scout.pubkey,
      proposedAt: now,
      approvedCostCents: null,
      updatedAt: now,
    },
  };
}

function companyInitiativeRecords(step: CofounderOnboardingStep = "brief"): {
  brief: MockCompanyInitiativeRecord;
  onboarding: MockCompanyInitiativeRecord;
} {
  const now = 1_791_273_600;
  const confirmed = step !== "brief";
  const content = {
    whatYouSell: "Social campaigns and brand work",
    whoBuys: "Local hospitality and retail teams",
    howYouTalk: "Warm, direct and never pushy.",
    sources: ["Website", "Colony history"],
  };
  const briefContent = {
    schemaVersion: 1,
    revision: confirmed ? 2 : 1,
    ...(confirmed
      ? {
          confirmed: content,
          confirmedByPubkey: TEST_IDENTITIES.tyler.pubkey,
          confirmedAt: now,
        }
      : { pending: content }),
    updatedByPubkey: TEST_IDENTITIES.tyler.pubkey,
    updatedAt: now,
  };
  const states = {
    brief: "todo",
    money: "todo",
    approvals: "todo",
    accounts: "todo",
    map: "todo",
  };
  let question: Record<string, unknown> | undefined;
  let accounts: Array<Record<string, unknown>> = [];
  let map: Record<string, unknown> | undefined;
  let status = "in_progress";
  if (step === "brief") {
    states.brief = "asked";
    question = { step: "brief", text: "Does this business brief sound right?" };
  } else {
    states.brief = "done";
  }
  if (["approvals", "account", "map", "complete"].includes(step)) {
    states.money = "done";
  } else if (step === "money") {
    states.money = "asked";
    question = { step: "money", text: "How should I spend money?" };
  }
  if (["account", "map", "complete"].includes(step)) {
    states.approvals = "done";
  } else if (step === "approvals") {
    states.approvals = "asked";
    question = {
      step: "approvals",
      text: "What else should wait for your OK?",
    };
  }
  if (step === "account") {
    states.accounts = "asked";
    question = {
      step: "accounts",
      text: "Please sign in to Pinterest.",
      account: {
        id: "pinterest",
        label: "Pinterest",
        url: "https://accounts.example.test/pinterest/login",
      },
    };
    accounts = [
      {
        id: "pinterest",
        label: "Pinterest",
        url: "https://accounts.example.test/pinterest/login",
        state: "asked",
      },
    ];
  }
  if (step === "map") {
    states.accounts = "done";
    accounts = [
      {
        id: "pinterest",
        label: "Pinterest",
        url: "https://accounts.example.test/pinterest/login",
        state: "done",
      },
    ];
    states.map = "asked";
  }
  if (step === "complete") {
    states.accounts = "skipped";
    states.map = "done";
    status = "complete";
  }
  if (step === "map" || step === "complete") {
    map = {
      entries: [
        {
          function: "sales",
          placement: "existing_agent",
          leadPubkey: REFERENCE_AGENTS.aya.pubkey,
        },
        { function: "operations", placement: "scout" },
        { function: "customer_success", placement: "empty" },
        { function: "finance", placement: "empty" },
        { function: "brand", placement: "empty" },
      ],
      firstFunction: "sales",
      approved: step === "complete",
    };
  }
  return {
    brief: { content: briefContent },
    onboarding: {
      content: {
        schemaVersion: 1,
        revision: 1,
        status,
        leadPubkey: REFERENCE_AGENTS.scout.pubkey,
        steps: [
          { step: "brief", state: states.brief },
          { step: "money", state: states.money },
          { step: "approvals", state: states.approvals },
          { step: "accounts", state: states.accounts },
          { step: "map", state: states.map },
        ],
        ...(question ? { question } : {}),
        accounts,
        ...(map ? { map } : {}),
        startedAt: now,
        resumeCount: 0,
        ...(status === "complete" ? { completedAt: now } : {}),
        updatedByPubkey: TEST_IDENTITIES.tyler.pubkey,
        updatedAt: now,
      },
    },
  };
}

async function boot(
  page: Page,
  options: {
    companyAccessRecords?: MockCompanyAccessRecord[];
    companyLeadStatus?: MockCofounderLeadStatus | null;
    companyFunctionActionErrors?: string[];
    companyNumbers?: MockNumbers;
    enabled?: boolean;
    identity?: RelayBridgeIdentity;
    businessName?: string;
    referenceWorkspace?: boolean;
    scoutDmChannelName?: string;
    withoutBrief?: boolean;
    step?: CofounderOnboardingStep;
    theme?: "light" | "dark";
  } = {},
) {
  const theme = options.theme ?? "light";
  await page.emulateMedia({ colorScheme: theme });
  await page.addInitScript(
    ({ selectedTheme, relayUrl }) => {
      const isDark = selectedTheme === "dark";
      const themeName = isDark ? "buzz-dark" : "buzz";
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
        `buzz-community-theme.v1:${"deadbeef".repeat(8)}:${encodeURIComponent(relayUrl)}`,
        JSON.stringify({
          version: 1,
          theme: themeName,
          accent: "#5B45BE",
          followSystem: false,
        }),
      );
    },
    { selectedTheme: theme, relayUrl: "ws://localhost:3000" },
  );
  const relaySecret = generateSecretKey();
  const initiative = companyInitiativeRecords(options.step);
  const referenceWorkspace = options.referenceWorkspace ?? true;
  await installMockBridge(
    page,
    {
      relaySelf: getPublicKey(relaySecret),
      companyMemberRelayPrivateKeyHex: bytesToHex(relaySecret),
      referenceWorkspace,
      colony3Sidebar: referenceWorkspace,
      ...(referenceWorkspace
        ? {
            ...(options.businessName
              ? { referenceWorkspaceBusinessName: options.businessName }
              : {}),
            colony3Extras: {
              ...(options.scoutDmChannelName
                ? {
                    patchChannels: [
                      {
                        id: REFERENCE_CHANNEL_IDS.scoutDm,
                        name: options.scoutDmChannelName,
                      },
                    ],
                  }
                : {}),
            },
          }
        : {}),
      relayRequiresMembership: true,
      relayRole: "owner",
      relayMembers: [
        { pubkey: TEST_IDENTITIES.tyler.pubkey, role: "owner" },
        { pubkey: REFERENCE_AGENTS.aya.pubkey, role: "member" },
        { pubkey: REFERENCE_AGENTS.scout.pubkey, role: "member" },
      ],
      cofounderSettings: {
        enabled: options.enabled ?? true,
        moneyPreference: "cheapest_first",
        approvalSwitches: APPROVAL_SWITCHES,
      },
      companyLeadStatus: options.companyLeadStatus,
      companyBriefRecord: options.withoutBrief ? null : initiative.brief,
      companyOnboardingRecord: initiative.onboarding,
      companyOnboardingAutoAdvance: true,
      companyFunctionActionErrors: options.companyFunctionActionErrors,
      companyPlans: [
        novemberPlan({
          scoutPubkey: REFERENCE_AGENTS.scout.pubkey,
          ayaPubkey: REFERENCE_AGENTS.aya.pubkey,
        }),
        esignSetupPlan({
          scoutPubkey: REFERENCE_AGENTS.scout.pubkey,
          ownerPubkey: TEST_IDENTITIES.tyler.pubkey,
        }),
      ],
      companyNumbers: options.companyNumbers,
      companyAccessRecords:
        options.companyAccessRecords ?? phase2AccessRecords(),
    },
    { identity: options.identity },
  );
  await page.addInitScript((batchDecisionKind) => {
    type WhatsAppMockState = {
      batchStatus: "pending" | "approved" | "rejected" | "completed";
      draftState: "draft" | "rejected" | "queued";
      commands: Array<{ kind: number; body: Record<string, unknown> }>;
    };
    type WhatsAppMockWindow = Window & {
      __BUZZ_E2E_WHATSAPP__?: (request: {
        action: "post" | "command";
        path?: string;
        body?: Record<string, unknown>;
        kind?: number;
      }) => Promise<unknown>;
      __BUZZ_E2E_WHATSAPP_STATE__?: {
        commands: Array<{ kind: number; body: Record<string, unknown> }>;
        connectCalls: number;
      };
    };
    const state: WhatsAppMockState = {
      batchStatus: "pending",
      draftState: "draft",
      commands: [],
    };
    const self = window as WhatsAppMockWindow;
    self.__BUZZ_E2E_WHATSAPP_STATE__ = {
      commands: state.commands,
      connectCalls: 0,
    };
    self.__BUZZ_E2E_WHATSAPP__ = async (request) => {
      if (request.action === "command") {
        const body = request.body ?? {};
        state.commands.push({ kind: request.kind ?? 0, body });
        if (request.kind === batchDecisionKind) {
          if (body.draftId === "cofounder-needs-you-draft") {
            state.draftState =
              body.decision === "reject" ? "rejected" : "queued";
          }
          if (body.batchId === "cofounder-needs-you-batch") {
            state.batchStatus =
              body.decision === "reject" ? "rejected" : "approved";
          }
        }
        return {};
      }

      const body = request.body ?? {};
      if (request.path !== "/api/whatsapp/query") return {};
      if (body.what === "status") {
        return {
          connected: true,
          connectionId: "e2e-cofounder",
          displayPhoneNumber: "+27 61 534 0627",
          verifiedName: "Horizon Labs",
          quality: "green",
          phoneStatus: "connected",
          dailyNewChatCap: 36,
          newChatsSentToday: 12,
          sendingPaused: false,
          pauseReason: null,
          issues: [],
          outreachChannelId: "e2e-whatsapp-outreach-channel",
          templatesSyncedAt: "2026-10-30T08:45:00+02:00",
          templates: [],
          webhookUrl: null,
        };
      }
      if (body.what === "batches") {
        const batches =
          state.batchStatus === "pending" ||
          body.state === undefined ||
          body.state === state.batchStatus
            ? [
                {
                  batchId: "cofounder-needs-you-batch",
                  status: state.batchStatus,
                  createdBy:
                    "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f",
                  template: "hl_vb",
                  language: "en",
                  rows: { total: 2, ready: 2, sent: 0, failed: 0, dropped: 0 },
                  createdAt: "2026-10-30T08:46:00+02:00",
                },
              ]
            : [];
        return { batches };
      }
      if (body.what === "drafts") {
        const drafts =
          state.draftState === "draft"
            ? [
                {
                  id: "cofounder-needs-you-draft",
                  waId: "27821237135",
                  direction: "out",
                  kind: "text",
                  text: "I have a few ideas for your November campaign. Would you like to see them?",
                  templateName: null,
                  state: state.draftState,
                  errorCode: null,
                  errorText: null,
                  eventId: null,
                  senderPubkey:
                    "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f",
                  createdAt: "2026-10-30T08:47:00+02:00",
                  to: "+27 82 123 7135",
                  params: [],
                },
              ]
            : [];
        return { drafts };
      }
      if (body.what === "batch") {
        return {
          batchId: "cofounder-needs-you-batch",
          status: state.batchStatus,
          createdBy:
            "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f",
          template: "hl_vb",
          language: "en",
          note: "November campaign introductions",
          materialLinks: [],
          decidedBy: null,
          decidedAt: null,
          createdAt: "2026-10-30T08:46:00+02:00",
          rows: [
            {
              idx: 0,
              to: "+27 82 123 7135",
              leadId: null,
              params: [],
              preview: "Hello from Horizon Labs",
              state: "ready",
              skipReason: null,
            },
            {
              idx: 1,
              to: "+27 82 987 6543",
              leadId: null,
              params: [],
              preview: "A note from Horizon Labs",
              state: "ready",
              skipReason: null,
            },
          ],
          counts: { total: 2, ready: 2, sent: 0, failed: 0, dropped: 0 },
        };
      }
      return {};
    };
  }, KIND_WHATSAPP_BATCH_DECISION);
}

async function capture(page: Page, name: string, fullPage = true) {
  await waitForAnimations(page);
  const themeName = name.includes("-dark") ? "buzz-dark" : "buzz";
  await expect
    .poll(() =>
      page.evaluate(() => {
        const root = document.documentElement;
        return {
          storedTheme: window.localStorage.getItem("buzz-theme"),
          dark: root.classList.contains("dark"),
          buzzTheme: root.getAttribute("data-buzz-theme"),
        };
      }),
    )
    .toEqual({
      storedTheme: themeName,
      dark: themeName === "buzz-dark",
      buzzTheme: themeName,
    });
  const proofDirectory = process.env.COF_UI_DEFECTS_PROOF_DIR;
  const file = proofDirectory
    ? resolve(proofDirectory, name)
    : test.info().outputPath(name);
  if (proofDirectory) {
    await mkdir(dirname(file), { recursive: true });
  }
  await page.screenshot({ fullPage, path: file });
}

async function expectNeedsYouContentLoaded(page: Page) {
  await expect(
    page.getByTestId(`cofounder-plan-${NOVEMBER_PLAN_ID}`),
  ).toBeVisible();
  await expect(
    page.getByTestId(`cofounder-plan-step-item-${ESIGN_PLAN_ID}-signup`),
  ).toBeVisible();
  await expect(
    page.getByText("Hire a Customer Success lead?", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("WhatsApp batch of 2 messages", { exact: true }),
  ).toBeVisible();
}

test.describe("Co-founder mode Phase 2", () => {
  test("enabled co-founder mode opens the Scout direct message from settings", async ({
    page,
  }) => {
    await page.clock.install({ time: new Date(1_791_273_600_000) });
    await boot(page, {
      scoutDmChannelName: "Lerato and Scout",
      step: "brief",
      withoutBrief: true,
      businessName: "Colony",
    });
    await page.goto("/#/settings?section=experimental");
    const card = page.getByTestId("cofounder-scout-entry");
    await expect(card).toBeVisible();
    await expect(
      card.getByText("Scout is getting to know Colony", {
        exact: true,
      }),
    ).toBeVisible();
    await expect(card).toContainText(
      "Reading your website, leads, chats and connected tools. Scout will message you a short brief to check.",
    );
    const avatar = page.getByTestId("cofounder-scout-avatar-image");
    await expect(avatar).toBeVisible();
    await expect(avatar).toHaveAttribute("alt", "Scout avatar");
    const openScout = page.getByTestId("cofounder-open-scout");
    await expect(openScout).toBeVisible();
    await expect(openScout).toBeEnabled();
    await openScout.click();
    await expect(page).toHaveURL(
      new RegExp(`#\\/channels\\/${REFERENCE_CHANNEL_IDS.scoutDm}$`, "u"),
    );
  });

  test("enabled co-founder mode creates a Scout direct message when none exists", async ({
    page,
  }) => {
    await page.clock.install({ time: new Date(1_791_273_600_000) });
    await boot(page, {
      referenceWorkspace: false,
      step: "brief",
      withoutBrief: true,
    });
    await page.goto("/#/settings?section=experimental");
    const openScout = page.getByTestId("cofounder-open-scout");
    await expect(openScout).toBeEnabled();
    await openScout.click();
    await expect(page).toHaveURL(/#\/channels\/[0-9a-f-]{36}$/u);
    await expect(page).not.toHaveURL(
      new RegExp(`#\\/channels\\/${REFERENCE_CHANNEL_IDS.scoutDm}$`, "u"),
    );
  });

  test("a stalled Scout card retries by publishing co-founder mode as enabled", async ({
    page,
  }) => {
    await page.clock.install({ time: new Date(1_791_273_600_000) });
    await boot(page, {
      identity: TEST_IDENTITIES.tyler,
      step: "brief",
      withoutBrief: true,
      businessName: "Colony",
    });
    await page.goto("/#/settings?section=experimental");

    const card = page.getByTestId("cofounder-scout-entry");
    await expect(
      card.getByText("Scout is getting to know Colony"),
    ).toBeVisible();
    await page.clock.fastForward(3 * 60 * 1000);
    await expect(card.getByText("Scout has not started yet")).toBeVisible();
    const retry = page.getByTestId("cofounder-retry-scout");
    await expect(retry).toHaveText("Try again");
    await retry.click();

    await expect
      .poll(() =>
        page.evaluate(() => {
          const events =
            (
              window as Window & {
                __BUZZ_E2E_SIGNED_EVENTS__?: Array<{
                  kind: number;
                  content?: string;
                }>;
              }
            ).__BUZZ_E2E_SIGNED_EVENTS__ ?? [];
          return events.some((event) => {
            if (event.kind !== 47060 || !event.content) return false;
            try {
              return (
                (
                  JSON.parse(event.content) as {
                    settings?: { enabled?: boolean };
                  }
                ).settings?.enabled === true
              );
            } catch {
              return false;
            }
          });
        }),
      )
      .toBe(true);
  });

  test("the durable wake status explains why Scout could not be reached", async ({
    page,
  }) => {
    await boot(page, {
      step: "brief",
      withoutBrief: true,
      companyLeadStatus: {
        schemaVersion: 1,
        state: "unreachable",
        problem: "several_scouts",
        detail: "The relay found more than one Scout.",
        attempts: 3,
        since: 1_791_273_600,
        updatedAt: 1_791_273_780,
      },
    });
    await page.goto("/#/settings?section=experimental");

    const card = page.getByTestId("cofounder-scout-entry");
    await expect(card.getByText("Scout could not be reached")).toBeVisible();
    await expect(card).toContainText("The relay found more than one Scout.");
    await expect(page.getByTestId("cofounder-retry-scout")).toBeVisible();
  });

  test("co-founder settings hides the getting to know card after Scout sends a brief", async ({
    page,
  }) => {
    await boot(page, { step: "brief" });
    await page.goto("/#/settings?section=experimental");
    await expect(page.getByTestId("cofounder-scout-entry")).toHaveCount(0);
  });

  test("co-founder settings hides the getting to know card after onboarding completes", async ({
    page,
  }) => {
    await boot(page, { step: "complete" });
    await page.goto("/#/settings?section=experimental");
    await expect(page.getByTestId("cofounder-scout-entry")).toHaveCount(0);
  });

  test("Scout guides onboarding one step at a time and then shows a plan in chat", async ({
    page,
  }) => {
    await boot(page, { identity: TEST_IDENTITIES.tyler });
    await page.goto(`/#/channels/${REFERENCE_CHANNEL_IDS.scoutDm}`);
    await expect(page.getByTestId("cofounder-chat-surface")).toBeVisible();
    await expect(page.getByTestId("cofounder-onboarding")).toBeVisible();
    await expect(
      page.getByRole("heading", { name: /what I know about Lerato Social/iu }),
    ).toBeVisible();
    await page.getByTestId("cofounder-brief-confirm").click();
    await expect(page.getByTestId("cofounder-money-best")).toBeVisible();
    await expect(page.getByTestId("cofounder-approval-save")).toHaveCount(0);

    await page.getByTestId("cofounder-money-best").click();
    await expect(page.getByTestId("cofounder-approval-save")).toBeVisible();
    await page
      .getByTestId("cofounder-approval-approveEveryDiscoveryCampaign")
      .uncheck();
    await page.getByTestId("cofounder-approval-save").click();
    await expect(page.getByTestId("cofounder-account-connected")).toBeVisible();
    await page.getByTestId("cofounder-account-connected").click();
    await expect(page.getByTestId("cofounder-map-approve")).toBeVisible();
    await page.getByTestId("cofounder-map-approve").click();
    await expect(
      page.getByRole("button", { name: "Finish onboarding" }),
    ).toBeVisible();
    await page.goto("/#/company/map");
    await expect(page.getByTestId("cofounder-map-sales")).toHaveAttribute(
      "data-state",
      "filled",
    );
    await expect(page.getByTestId("cofounder-map-sales")).toHaveAttribute(
      "aria-label",
      /Sales, led by/iu,
    );
    await expect(page.getByTestId("cofounder-map-operations")).toHaveAttribute(
      "data-state",
      "covered_by_scout",
    );
    await expect(page.getByTestId("cofounder-map-operations")).toHaveAttribute(
      "aria-label",
      "Operations, covered by Scout",
    );
    await page.goto(`/#/channels/${REFERENCE_CHANNEL_IDS.scoutDm}`);
    await expect(
      page.getByRole("button", { name: "Finish onboarding" }),
    ).toBeVisible();
    await page.getByRole("button", { name: "Finish onboarding" }).click();

    const plan = page.getByTestId(`cofounder-plan-${NOVEMBER_PLAN_ID}`);
    await expect(plan).toBeVisible();
    await expect(plan).toContainText("5 won");
    await plan.getByTestId(`cofounder-plan-edit-${NOVEMBER_PLAN_ID}`).click();
    await page
      .getByTestId(`cofounder-plan-title-${NOVEMBER_PLAN_ID}`)
      .fill("A revised guesthouse plan");
    await page.getByRole("button", { name: "Save changes" }).click();
    await expect(plan).toContainText("A revised guesthouse plan");
    await expect(plan).toContainText("Find 60 guesthouses");
    await plan
      .getByTestId(`cofounder-plan-approve-${NOVEMBER_PLAN_ID}`)
      .click();
    await expect(plan).toContainText("Approved");
  });

  test("an approved company map retries function slot writes before finishing", async ({
    page,
  }) => {
    await boot(page, {
      companyFunctionActionErrors: ["Temporary function slot write failure."],
      identity: TEST_IDENTITIES.tyler,
      step: "map",
    });
    await page.goto(`/#/channels/${REFERENCE_CHANNEL_IDS.scoutDm}`);
    await page.getByTestId("cofounder-map-approve").click();
    await expect(page.getByRole("alert")).toContainText(
      "Temporary function slot write failure.",
    );

    await page.getByRole("button", { name: "Finish onboarding" }).click();
    await expect(
      page.getByTestId(`cofounder-plan-${NOVEMBER_PLAN_ID}`),
    ).toBeVisible();
    await page.goto("/#/company/map");
    await expect(page.getByTestId("cofounder-map-sales")).toHaveAttribute(
      "data-state",
      "filled",
    );
    await expect(page.getByTestId("cofounder-map-operations")).toHaveAttribute(
      "data-state",
      "covered_by_scout",
    );
  });

  test("Scout plan can be rejected", async ({ page }) => {
    await boot(page, {
      identity: TEST_IDENTITIES.tyler,
      step: "complete",
    });
    await page.goto(`/#/channels/${REFERENCE_CHANNEL_IDS.scoutDm}`);
    const plan = page.getByTestId(`cofounder-plan-${NOVEMBER_PLAN_ID}`);
    await expect(plan).toBeVisible();
    await plan.getByTestId(`cofounder-plan-reject-${NOVEMBER_PLAN_ID}`).click();
    await expect(plan).toContainText("Rejected");
  });

  test("primary plan actions keep readable contrast in light and dark themes", async ({
    page,
  }) => {
    for (const theme of ["light", "dark"] as const) {
      await boot(page, { step: "complete", theme });
      await page.goto(`/#/channels/${REFERENCE_CHANNEL_IDS.scoutDm}`);
      const approve = page.getByTestId(
        `cofounder-plan-approve-${NOVEMBER_PLAN_ID}`,
      );
      await expect(approve).toBeVisible();
      const ratio = await approve.evaluate((button) => {
        const parse = (value: string) =>
          value.match(/[\d.]+/gu)?.map(Number) ?? [];
        const toLinear = (channel: number) => {
          const normalized = channel / 255;
          return normalized <= 0.04045
            ? normalized / 12.92
            : ((normalized + 0.055) / 1.055) ** 2.4;
        };
        const luminance = (color: string) => {
          const [red, green, blue] = parse(color);
          return (
            0.2126 * toLinear(red ?? 0) +
            0.7152 * toLinear(green ?? 0) +
            0.0722 * toLinear(blue ?? 0)
          );
        };
        const style = window.getComputedStyle(button);
        const foreground = luminance(style.color);
        const background = luminance(style.backgroundColor);
        return (
          (Math.max(foreground, background) + 0.05) /
          (Math.min(foreground, background) + 0.05)
        );
      });
      expect(ratio, `${theme} primary action contrast`).toBeGreaterThanOrEqual(
        4.5,
      );
    }

    await page.setViewportSize({ width: 390, height: 844 });
    const tableWrap = page.locator(".cf2-plan-table-wrap");
    await expect
      .poll(() =>
        tableWrap.evaluate(
          (element) => element.scrollWidth - element.clientWidth,
        ),
      )
      .toBeLessThanOrEqual(0);
    await expect(page.locator(".cf2-step-mobile-meta").first()).toBeVisible();
  });

  test("Needs you approves or rejects items and opens the exact prepared page", async ({
    page,
  }) => {
    await installBrowserHostFake(page);
    await boot(page, { identity: TEST_IDENTITIES.tyler });
    await page.goto("/#/today/needs-you");
    await expect(page.getByTestId("cofounder-needs-you-screen")).toBeVisible();
    await expect(
      page.getByRole("heading", { name: "Needs you", exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole("heading", {
        name: "Get 5 Horizon Labs clients in November",
        exact: true,
      }),
    ).toHaveCount(1);

    await page
      .getByTestId(`cofounder-plan-approve-${NOVEMBER_PLAN_ID}`)
      .click();
    await expect(
      page.getByTestId(`cofounder-plan-${NOVEMBER_PLAN_ID}`),
    ).toHaveCount(0);
    await page
      .getByTestId(
        `cofounder-plan-step-action-approve-${ESIGN_PLAN_ID}-template`,
      )
      .click();
    await expect(
      page.getByTestId(`cofounder-plan-step-item-${ESIGN_PLAN_ID}-template`),
    ).toHaveCount(0);
    const paymentGap = page.getByTestId(
      `cofounder-capability-gap-${PAYMENT_GAP_ID}`,
    );
    await expect(paymentGap).toContainText("Connect a payment link account");
    await page.getByTestId(`cofounder-gap-approve-${PAYMENT_GAP_ID}`).click();
    await expect(page.getByTestId("work-area-browser")).toBeVisible();
    await expect(page.getByTestId("browser-address")).toHaveValue(
      "https://accounts.example.test/payment-link/login",
    );
  });

  test("Needs you reads WhatsApp batches and drafts and submits draft decisions", async ({
    page,
  }) => {
    await boot(page, { identity: TEST_IDENTITIES.tyler });
    await page.goto("/#/today/needs-you");

    const batch = page.getByTestId(
      `cofounder-live-needs-you-wa-batch:${NEEDS_YOU_WA_BATCH_ID}`,
    );
    const draft = page.getByTestId(`cofounder-draft-${NEEDS_YOU_WA_DRAFT_ID}`);
    await expect(batch).toContainText("WhatsApp batch of 2 messages");
    await expect(draft).toContainText(
      "I have a few ideas for your November campaign.",
    );
    await page
      .getByTestId(`cofounder-discard-draft-${NEEDS_YOU_WA_DRAFT_ID}`)
      .click();
    await expect(draft).toHaveCount(0);
    await expect
      .poll(() =>
        page.evaluate(() => {
          const testWindow = window as Window & {
            __BUZZ_E2E_WHATSAPP_STATE__?: {
              commands: Array<{
                kind: number;
                body: Record<string, unknown>;
              }>;
            };
          };
          return (
            testWindow.__BUZZ_E2E_WHATSAPP_STATE__?.commands.at(-1) ?? null
          );
        }),
      )
      .toEqual({
        kind: KIND_WHATSAPP_BATCH_DECISION,
        body: { v: 1, draftId: NEEDS_YOU_WA_DRAFT_ID, decision: "reject" },
      });

    await batch.getByRole("button", { name: /Review batch/u }).click();
    await expect(page.getByTestId("whatsapp-batch-page")).toBeVisible();
    const batchPage = page.getByTestId("whatsapp-batch-page");
    await expect(batchPage).toContainText("2 businesses");
    await expect(batchPage).toContainText("Guesthouses · 2");
  });

  test("Needs you completes real plan and capability-gap steps", async ({
    page,
  }) => {
    await installBrowserHostFake(page);
    const approvedGap = capabilityGapRecord();
    approvedGap.content = {
      ...approvedGap.content,
      state: "approved",
      approvedCostCents: 0,
    };
    await boot(page, {
      companyAccessRecords: [approvedGap],
      identity: TEST_IDENTITIES.tyler,
    });
    await page.goto("/#/today/needs-you");
    await page
      .getByTestId(`cofounder-plan-step-action-done-${ESIGN_PLAN_ID}-signup`)
      .click();
    await expect(
      page.getByTestId(`cofounder-plan-step-item-${ESIGN_PLAN_ID}-signup`),
    ).toHaveCount(0);
    await page
      .getByTestId(`cofounder-gap-open-browser-${CAPABILITY_GAP_ID}`)
      .click();
    await expect(page.getByTestId("work-area-browser")).toBeVisible();
    await expect(page.getByTestId("browser-address")).toHaveValue(
      "https://accounts.example.test/esign/login",
    );
    await page.getByRole("button", { name: "Today", exact: true }).click();
    await page.getByTestId("today-needs-you-link").click();
    await expect(
      page.getByTestId(`cofounder-gap-open-browser-${CAPABILITY_GAP_ID}`),
    ).toBeVisible();
    await page.getByTestId(`cofounder-gap-done-${CAPABILITY_GAP_ID}`).click();
    await expect(
      page.getByTestId(`cofounder-capability-gap-${CAPABILITY_GAP_ID}`),
    ).toHaveCount(0);
  });

  test("Access shows pending requests and persists revocation for the selected tool", async ({
    page,
  }) => {
    await installBrowserHostFake(page);
    await boot(page, { identity: TEST_IDENTITIES.tyler });
    await page.goto("/#/company/access");
    await expect(page.getByTestId("cofounder-access-screen")).toBeVisible();
    await expect(page.getByTestId("sidebar-nav-company-content")).toBeHidden();
    await expect(
      page.getByTestId("sidebar-nav-company-toggle"),
    ).toHaveAttribute("data-current", "true");
    await expect(page.getByTestId("sidebar-activity-button")).toHaveAttribute(
      "data-active",
      "false",
    );
    await page.getByTestId("sidebar-nav-company-toggle").click();
    await expect(page.getByTestId("sidebar-company-access")).toHaveAttribute(
      "aria-current",
      "page",
    );
    const pendingPaymentGap = page.getByTestId(
      `cofounder-access-pending-${PAYMENT_GAP_ID}`,
    );
    await expect(pendingPaymentGap).toContainText(
      "Connect a payment link account",
    );
    await expect(pendingPaymentGap).toContainText(
      "Free to join, a fee per payment",
    );

    await page.getByTestId("cofounder-access-revoke-whatsapp").click();
    await expect(page.getByTestId("cofounder-access-whatsapp")).toContainText(
      "Revoked",
    );
    await expect(
      page.getByTestId("cofounder-access-revoke-whatsapp"),
    ).toHaveCount(0);
    await expect(page.getByTestId("cofounder-access-gmail")).toBeVisible();

    await page.goto("/#/company/access");
    await page
      .getByTestId(`cofounder-access-approve-${PAYMENT_GAP_ID}`)
      .click();
    await expect(page.getByTestId("work-area-browser")).toBeVisible();
    await expect(page.getByTestId("browser-address")).toHaveValue(
      "https://accounts.example.test/payment-link/login",
    );
    await page.goto("/#/today/needs-you");
    await expect(
      page.getByTestId(`cofounder-gap-open-browser-${PAYMENT_GAP_ID}`),
    ).toBeVisible();
  });

  test("capability gaps move from approval to the built-in browser step", async ({
    page,
  }) => {
    await installBrowserHostFake(page);
    await boot(page, {
      companyAccessRecords: [capabilityGapRecord()],
      identity: TEST_IDENTITIES.tyler,
    });
    await page.goto("/#/today/needs-you");

    const gap = page.getByTestId(
      `cofounder-capability-gap-${CAPABILITY_GAP_ID}`,
    );
    await expect(gap).toContainText("Connect the e-sign account");
    await page
      .getByTestId(`cofounder-gap-approve-${CAPABILITY_GAP_ID}`)
      .click();
    const browserTab = page.getByRole("tabpanel", {
      name: /accounts\.example\.test\/esign/u,
    });
    await expect(browserTab).toBeVisible();
    await expect(browserTab.getByTestId("browser-address")).toHaveValue(
      "https://accounts.example.test/esign/login",
    );

    await page.getByRole("button", { name: "Today", exact: true }).click();
    await page.getByTestId("today-needs-you-link").click();
    await expect(
      page.getByTestId(`cofounder-gap-open-browser-${CAPABILITY_GAP_ID}`),
    ).toBeVisible();
    await page
      .getByTestId(`cofounder-gap-open-browser-${CAPABILITY_GAP_ID}`)
      .click();
    await expect(browserTab).toBeVisible();
    await expect(browserTab.getByTestId("browser-address")).toHaveValue(
      "https://accounts.example.test/esign/login",
    );

    await page.getByRole("button", { name: "Today", exact: true }).click();
    await page.getByTestId("today-needs-you-link").click();
    await page.getByTestId(`cofounder-gap-done-${CAPABILITY_GAP_ID}`).click();
    await expect(gap).toHaveCount(0);
  });

  test("declining a capability gap removes it from both approval surfaces", async ({
    page,
  }) => {
    await boot(page, {
      companyAccessRecords: [capabilityGapRecord()],
      identity: TEST_IDENTITIES.tyler,
    });
    await page.goto("/#/today/needs-you");
    await page
      .getByTestId(`cofounder-gap-decline-${CAPABILITY_GAP_ID}`)
      .click();

    await expect(
      page.getByTestId(`cofounder-capability-gap-${CAPABILITY_GAP_ID}`),
    ).toHaveCount(0);
    await page.goto("/#/company/access");
    await expect(
      page.getByTestId(`cofounder-access-pending-${CAPABILITY_GAP_ID}`),
    ).toHaveCount(0);
  });

  test("Phase 2 surfaces disappear when co-founder mode is off", async ({
    page,
  }) => {
    await boot(page, { enabled: false });
    await page.goto("/#/company/access");
    await expect(page.getByTestId("cofounder-access-screen")).toHaveCount(0);
    await expect(page).toHaveURL(/#\/today$/u);
    await expect(page.getByTestId("today-screen")).toBeVisible();
  });

  test("captures each approved surface at desktop and mobile sizes in light and dark", async ({
    browser,
  }) => {
    test.setTimeout(180_000);
    const cases = [
      {
        surface: "onboarding",
        route: `/#/channels/${REFERENCE_CHANNEL_IDS.scoutDm}`,
        step: "map" as const,
      },
      {
        surface: "settings",
        route: "/#/settings?section=experimental",
        step: "brief" as const,
        withoutBrief: true,
        businessName: "Colony",
      },
      {
        surface: "settings-stalled",
        route: "/#/settings?section=experimental",
        step: "brief" as const,
        withoutBrief: true,
        businessName: "Colony",
        stalled: true,
      },
      {
        surface: "plan",
        route: `/#/channels/${REFERENCE_CHANNEL_IDS.scoutDm}`,
        step: "complete" as const,
      },
      {
        surface: "needs-you",
        route: "/#/today/needs-you",
        step: "brief" as const,
      },
      { surface: "access", route: "/#/company/access", step: "brief" as const },
    ];

    for (const theme of ["light", "dark"] as const) {
      for (const viewport of [
        { name: "1440", width: 1440, height: 900 },
        { name: "390", width: 390, height: 844 },
      ]) {
        for (const item of cases) {
          const screenshotContext = await browser.newContext();
          try {
            const page = await screenshotContext.newPage();
            if (item.surface.startsWith("settings")) {
              await page.clock.install({
                time: new Date(1_791_273_600_000),
              });
            }
            const captureHeight =
              item.surface === "onboarding"
                ? 2400
                : item.surface === "plan"
                  ? 1600
                  : item.surface === "needs-you"
                    ? viewport.width === 390
                      ? 3500
                      : 2500
                    : item.surface === "access"
                      ? viewport.width === 390
                        ? 2100
                        : 1300
                      : viewport.height;
            await page.setViewportSize({
              width: viewport.width,
              height: captureHeight,
            });
            const needsYouPreview = item.surface === "needs-you";
            await boot(page, {
              theme,
              step: item.step,
              ...(item.surface.startsWith("settings")
                ? { businessName: "Colony" }
                : {}),
              ...(item.surface.startsWith("settings")
                ? { withoutBrief: true }
                : {}),
              identity: TEST_IDENTITIES.tyler,
              ...(needsYouPreview
                ? {
                    companyNumbers: NEEDS_YOU_NUMBERS,
                    companyAccessRecords: needsYouAccessRecords(),
                  }
                : {}),
            });
            await page.goto(item.route);
            if (item.surface === "settings-stalled") {
              await page.clock.fastForward(3 * 60 * 1000);
            }
            const expected = (target: Page) =>
              item.surface === "onboarding"
                ? target.getByTestId("cofounder-onboarding")
                : item.surface === "plan"
                  ? target.getByTestId(`cofounder-plan-${NOVEMBER_PLAN_ID}`)
                  : item.surface === "needs-you"
                    ? target.getByTestId("cofounder-needs-you-screen")
                    : item.surface === "access"
                      ? target.getByTestId("cofounder-access-screen")
                      : item.surface === "settings-stalled"
                        ? target.getByTestId("cofounder-retry-scout")
                        : target.getByTestId("cofounder-open-scout");
            await expect(expected(page)).toBeVisible();
            if (item.surface === "settings-stalled") {
              await expect(
                page.getByText("Scout has not started yet"),
              ).toBeVisible();
            }
            if (item.surface === "onboarding") {
              await expect(
                page.getByText("pinterest login is open in the browser"),
              ).toBeVisible();
            }
            if (needsYouPreview) await expectNeedsYouContentLoaded(page);
            await capture(
              page,
              `cofounder-phase2-${item.surface}-${viewport.name}-${theme}.png`,
            );
            if (
              viewport.name === "390" &&
              item.surface !== "settings" &&
              item.surface !== "settings-stalled"
            ) {
              const viewportContext = await browser.newContext({
                viewport: { width: 390, height: 844 },
              });
              try {
                const viewportPage = await viewportContext.newPage();
                await boot(viewportPage, {
                  theme,
                  step: item.step,
                  ...(item.surface === "settings"
                    ? { withoutBrief: true }
                    : {}),
                  identity: TEST_IDENTITIES.tyler,
                  ...(needsYouPreview
                    ? {
                        companyNumbers: NEEDS_YOU_NUMBERS,
                        companyAccessRecords: needsYouAccessRecords(),
                      }
                    : {}),
                });
                await viewportPage.goto(item.route);
                await expect(expected(viewportPage)).toBeVisible();
                if (item.surface === "onboarding") {
                  await expect(
                    viewportPage.getByText(
                      "pinterest login is open in the browser",
                    ),
                  ).toBeVisible();
                }
                if (needsYouPreview)
                  await expectNeedsYouContentLoaded(viewportPage);
                await capture(
                  viewportPage,
                  `cofounder-phase2-${item.surface}-390-${theme}-viewport.png`,
                  false,
                );
              } finally {
                await viewportContext.close();
              }
            }
          } finally {
            await screenshotContext.close();
          }
        }
      }
    }
  });
});
