import { mkdirSync } from "node:fs";
import { resolve } from "node:path";
import { expect, test, type Page } from "@playwright/test";
import { bytesToHex, hexToBytes } from "@noble/hashes/utils.js";
import {
  finalizeEvent,
  generateSecretKey,
  getPublicKey,
} from "nostr-tools/pure";

import { KIND_HIRE_HEAD } from "../../src/shared/constants/kinds";
import type { RelayEvent } from "../../src/shared/api/types";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const PERSONA_ID = "company-role-hospitality-research";
const HIRE_ASK_ID = "7245ba1a-e078-42ef-b896-00be34a94f11";
const HIRE_ASK_HIRE_ID = "7916ba1a-e078-42ef-b896-00be34a94f12";
const GENERAL_CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";

function newRelayKey() {
  const secretKey = generateSecretKey();
  return {
    privateKeyHex: bytesToHex(secretKey),
    relaySelf: getPublicKey(secretKey),
  };
}

test("owner hires from a role and the configured draft survives reload", async ({
  page,
}) => {
  const relay = newRelayKey();
  await installMockBridge(page, {
    relaySelf: relay.relaySelf,
    companyHireRelayPrivateKeyHex: relay.privateKeyHex,
    relayRequiresMembership: true,
    personas: [
      {
        id: PERSONA_ID,
        displayName: "Hospitality researcher",
        systemPrompt: "Research hospitality accounts and report findings.",
        isActive: true,
        runtime: "buzz-agent",
        provider: "openai",
        model: "gpt-5.5",
        envVars: { BUZZ_AGENT_PROVIDER: "openai" },
        companyRole: {
          job: "Hospitality research",
          skills: ["Market research", "Synthesis"],
          tools: [{ name: "Web search", risk: "low" }],
          workerMenu: ["buzz-agent"],
        },
      },
    ],
    discoverAgentModels: {
      models: [{ id: "gpt-5.5", name: "GPT-5.5" }],
      supportsSwitching: true,
      selectedModel: "gpt-5.5",
    },
  });

  await page.goto("/#/team");
  await page.getByRole("button", { name: "Hire an agent" }).click();
  await expect(page.getByTestId(`hire-role-${PERSONA_ID}`)).toBeVisible();
  await expect(page.getByRole("button", { name: "Edit pack" })).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Open role pack" }),
  ).toHaveCount(0);
  await page
    .getByTestId(`hire-role-${PERSONA_ID}`)
    .getByRole("button", { name: "Configure" })
    .click();

  const name = page.getByTestId("hire-employee-name");
  const title = page.getByTestId("hire-title");
  const instructionScope = page.getByTestId("hire-respond-to");
  const workerModel = page.locator(
    '[data-testid="hire-ai-model-option"]:checked',
  );
  await expect(name).toBeVisible();
  await name.fill("Mina");
  await title.fill("Hospitality research lead");
  await instructionScope.selectOption("anyone");
  await expect(workerModel).toHaveValue(
    JSON.stringify(["buzz-agent", "openai", "gpt-5.5"]),
  );

  await page.reload();
  await expect(page.getByTestId("hire-employee-name")).toHaveValue("Mina");
  await expect(page.getByTestId("hire-title")).toHaveValue(
    "Hospitality research lead",
  );
  await expect(page.getByTestId("hire-respond-to")).toHaveValue("anyone");
  await expect(
    page.locator('[data-testid="hire-ai-model-option"]:checked'),
  ).toHaveValue(JSON.stringify(["buzz-agent", "openai", "gpt-5.5"]));
  await page.getByTestId("hire-review").click();

  await expect(page.getByTestId("hire-review-form")).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Review hire" }),
  ).toBeVisible();
  await expect(
    page.getByText("Review the exact scope before hiring."),
  ).toBeVisible();
  await page.getByTestId("hire-founder-confirm").check();
  await page.getByTestId("hire-approve").click();

  await expect(page.getByTestId("hire-success")).toBeVisible({
    timeout: 15_000,
  });
  await expect(page.getByText("Mina joined the team.")).toBeVisible();
  await expect(page.getByText(/Hello team\. I’ll coordinate/)).toBeVisible();
});

test("a business with no role personas is offered the built-in roles", async ({
  page,
}) => {
  const relay = newRelayKey();
  await installMockBridge(page, {
    relaySelf: relay.relaySelf,
    companyHireRelayPrivateKeyHex: relay.privateKeyHex,
    relayRequiresMembership: true,
    personas: [],
    discoverAgentModels: {
      models: [{ id: "gpt-5.5", name: "GPT-5.5" }],
      supportsSwitching: true,
      selectedModel: "gpt-5.5",
    },
  });
  await page.goto("/#/hire/roles");

  const names = [
    "Hospitality researcher",
    "Social media manager",
    "Bookkeeper",
    "Business development",
    "Writer",
  ];
  for (const name of names) {
    await expect(
      page
        .getByRole("region", { name: "Role catalog" })
        .getByRole("heading", { name }),
    ).toBeVisible();
  }
  await expect(
    page.getByRole("heading", { name: "Your role catalog starts here" }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Create role pack" }),
  ).toHaveCount(0);
  const socialCard = page.getByTestId(
    "hire-role-built-in-role-social-media-manager",
  );
  await expect(socialCard).toContainText(
    "Plans, writes and schedules social content for the company's clients.",
  );

  const rolePersonas = () =>
    page.evaluate(async () => {
      const invoke = (
        window as Window & {
          __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
            command: string,
            payload?: unknown,
          ) => Promise<unknown>;
        }
      ).__BUZZ_E2E_INVOKE_MOCK_COMMAND__;
      const personas = (await invoke?.("list_personas", {})) as Array<{
        display_name: string;
        company_role: unknown;
      }>;
      return personas
        .filter((persona) => persona.company_role)
        .map((persona) => ({
          display_name: persona.display_name,
          company_role: persona.company_role,
        }));
    });
  expect(await rolePersonas()).toEqual([]);

  // A double click on a built-in role creates its persona once and reaches Configure.
  await socialCard.getByRole("button", { name: "Configure" }).dblclick();
  await expect(page.getByTestId("hire-employee-name")).toBeVisible();
  await expect(page).toHaveURL(/#\/hire\/configure\?.*personaId=/);
  const configuredRoles = await rolePersonas();
  expect(configuredRoles.map((persona) => persona.display_name)).toEqual([
    "Social media manager",
  ]);
  expect(configuredRoles[0]?.company_role).toEqual(
    expect.objectContaining({
      job: "Client social content",
      skills: ["Content planning", "Captions", "Scheduling"],
      tools: [
        { name: "Web search", risk: "low" },
        { name: "Social publishing", risk: "high" },
      ],
    }),
  );

  // The role is now the business's own: it is listed once, and a second hire
  // reuses the persona instead of adding another.
  await page.goto("/#/hire/roles");
  await expect(
    page
      .getByRole("region", { name: "Role catalog" })
      .getByRole("heading", { name: "Social media manager" }),
  ).toHaveCount(1);
  await expect(
    page.getByTestId("hire-role-built-in-role-social-media-manager"),
  ).toHaveCount(0);
  await page
    .locator('[data-testid^="hire-role-"]', {
      hasText: "Social media manager",
    })
    .getByRole("button", { name: "Configure" })
    .click();
  await expect(page.getByTestId("hire-employee-name")).toBeVisible();
  const reusedRoles = await rolePersonas();
  expect(reusedRoles.map((persona) => persona.display_name)).toEqual([
    "Social media manager",
  ]);
});

test("a member cannot open the role catalog", async ({ page }) => {
  await installMockBridge(page, {
    relayRequiresMembership: true,
    relayRole: "member",
    personas: [],
  });
  await page.goto("/#/hire/roles");

  await expect(
    page.getByRole("heading", {
      name: "You do not have permission for this action",
    }),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "Configure" })).toHaveCount(0);
});

const BUILT_IN_ROLE_NAMES = [
  "Hospitality researcher",
  "Social media manager",
  "Bookkeeper",
  "Business development",
  "Writer",
];
const CONFIGURE_HIRE_ID = "0b6cbd5e-5c0e-4a3e-9f47-6c2a0a7f2d11";

async function installBlankConfigureMock(
  page: Page,
  mock: Record<string, unknown> = {},
) {
  const relay = newRelayKey();
  await installMockBridge(page, {
    relaySelf: relay.relaySelf,
    companyHireRelayPrivateKeyHex: relay.privateKeyHex,
    relayRequiresMembership: true,
    personas: [],
    discoverAgentModels: {
      models: [{ id: "gpt-5.5", name: "GPT-5.5" }],
      supportsSwitching: true,
      selectedModel: "gpt-5.5",
    },
    ...mock,
  });
}

for (const roleName of BUILT_IN_ROLE_NAMES) {
  test(`Configure this role opens the form for ${roleName}`, async ({
    page,
  }) => {
    await installBlankConfigureMock(page);
    await page.goto("/#/hire/roles");
    await page
      .getByRole("region", { name: "Role catalog" })
      .locator('[data-testid^="hire-role-"]', { hasText: roleName })
      .getByRole("button", { name: "Configure" })
      .click();
    await expect(page.getByTestId("hire-employee-name")).toBeVisible();
    await expect(page.getByTestId("hire-configure-state")).toHaveCount(0);
  });
}

test("Business development configures twice through its single materialized persona", async ({
  page,
}) => {
  await installBlankConfigureMock(page);
  await page.goto("/#/hire/roles");
  const builtInRole = page.getByTestId(
    "hire-role-built-in-role-business-development",
  );
  await expect(builtInRole).toBeVisible();
  await builtInRole.getByRole("button", { name: "Configure" }).dblclick();
  await expect(page.getByTestId("hire-employee-name")).toBeVisible();
  await expect(page.getByTestId("hire-configure-state")).toHaveCount(0);

  const rolePersonas = await page.evaluate(async () => {
    const invoke = (
      window as Window & {
        __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
          command: string,
          payload?: unknown,
        ) => Promise<unknown>;
      }
    ).__BUZZ_E2E_INVOKE_MOCK_COMMAND__;
    return (await invoke?.("list_personas", {})) as Array<{
      id: string;
      display_name: string;
      company_role: unknown;
    }>;
  });
  const businessDevelopment = rolePersonas.filter(
    (persona) =>
      persona.display_name === "Business development" && persona.company_role,
  );
  expect(businessDevelopment).toHaveLength(1);

  await page.goto("/#/hire/roles");
  await page
    .getByTestId(`hire-role-${businessDevelopment[0].id}`)
    .getByRole("button", { name: "Configure" })
    .click();
  await expect(page.getByTestId("hire-employee-name")).toBeVisible();
  await expect(page.getByTestId("hire-configure-state")).toHaveCount(0);
});

test("captures the Business development role flow at both widths and themes", async ({
  browser,
}) => {
  test.skip(
    process.env.COF_UI_DEFECT_CAPTURE !== "1",
    "capture is requested for proof delivery",
  );
  test.setTimeout(120_000);
  const output = resolve("/private/tmp/cof-ui-defects-proof/screenshots");
  mkdirSync(output, { recursive: true });

  for (const theme of ["light", "dark"] as const) {
    for (const viewport of [
      { name: "1440", width: 1440, height: 900 },
      { name: "390", width: 390, height: 844 },
    ]) {
      const context = await browser.newContext();
      try {
        const page = await context.newPage();
        await page.setViewportSize(viewport);
        await page.emulateMedia({ colorScheme: theme });
        await installBlankConfigureMock(page);
        await page.addInitScript((selectedTheme) => {
          const themeName = selectedTheme === "dark" ? "buzz-dark" : "buzz";
          window.localStorage.removeItem("buzz-theme-cache");
          window.localStorage.setItem("buzz-theme", themeName);
          window.localStorage.setItem("buzz-follow-system", "false");
          window.localStorage.setItem("buzz-accent-color", "#5B45BE");
        }, theme);
        await page.goto("/#/hire/roles");
        const role = page.getByTestId(
          "hire-role-built-in-role-business-development",
        );
        await expect(role).toBeVisible();
        await waitForAnimations(page);
        await page.screenshot({
          path: resolve(
            output,
            `c4-business-development-catalog-${viewport.name}-${theme}.png`,
          ),
          fullPage: true,
        });
        await role.getByRole("button", { name: "Configure" }).click();
        await expect(page.getByTestId("hire-employee-name")).toBeVisible();
        await expect(page.getByTestId("hire-configure-state")).toHaveCount(0);
        await waitForAnimations(page);
        await page.screenshot({
          path: resolve(
            output,
            `c4-business-development-configure-${viewport.name}-${theme}.png`,
          ),
          fullPage: true,
        });
      } finally {
        await context.close();
      }
    }
  }
});

test("Configure says the role could not be loaded and goes back to roles", async ({
  page,
}) => {
  await installBlankConfigureMock(page);
  await page.goto(
    `/#/hire/configure?personaId=gone-role&hireId=${CONFIGURE_HIRE_ID}`,
  );
  const state = page.getByTestId("hire-configure-state");
  await expect(state).toContainText("This role could not be loaded");
  await expect(page.getByTestId("hire-employee-name")).toHaveCount(0);
  await state.getByRole("button", { name: "Back to roles" }).click();
  await expect(page.getByTestId("hire-roles")).toBeVisible();
});

test("Configure shows a recoverable error when the team read fails", async ({
  page,
}) => {
  await installBlankConfigureMock(page, {
    personas: [
      {
        id: PERSONA_ID,
        displayName: "Hospitality researcher",
        systemPrompt: "Research hospitality accounts and report findings.",
        isActive: true,
        runtime: "buzz-agent",
        companyRole: {
          job: "Hospitality research",
          skills: ["Market research"],
          tools: [{ name: "Web search", risk: "low" }],
          workerMenu: ["buzz-agent"],
        },
      },
    ],
    commandErrors: { list_relay_agents: "relay returned 500" },
  });
  await page.goto(
    `/#/hire/configure?personaId=${PERSONA_ID}&hireId=${CONFIGURE_HIRE_ID}`,
  );
  const state = page.getByTestId("hire-configure-state");
  await expect(state).toContainText("This information could not load", {
    timeout: 20_000,
  });
  await expect(
    state.getByRole("button", { name: "Back to roles" }),
  ).toBeVisible();

  await page.evaluate(() => {
    const config = (
      window as Window & {
        __BUZZ_E2E__?: { mock?: { commandErrors?: Record<string, string> } };
      }
    ).__BUZZ_E2E__;
    if (config?.mock) config.mock.commandErrors = {};
  });
  await state.getByRole("button", { name: "Try again" }).click();
  await expect(page.getByTestId("hire-employee-name")).toBeVisible();
});

test("Configure shows a loading state while the team read is pending, then a timeout", async ({
  page,
}) => {
  await page.clock.install();
  await installBlankConfigureMock(page, {
    personas: [
      {
        id: PERSONA_ID,
        displayName: "Hospitality researcher",
        systemPrompt: "Research hospitality accounts and report findings.",
        isActive: true,
        runtime: "buzz-agent",
        companyRole: {
          job: "Hospitality research",
          skills: ["Market research"],
          tools: [{ name: "Web search", risk: "low" }],
          workerMenu: ["buzz-agent"],
        },
      },
    ],
    commandDelaysMs: { list_managed_agents: 600_000 },
  });
  await page.goto(
    `/#/hire/configure?personaId=${PERSONA_ID}&hireId=${CONFIGURE_HIRE_ID}`,
  );
  const state = page.getByTestId("hire-configure-state");
  await expect(state).toContainText("Loading the employee form");
  await expect(page.getByTestId("hire-employee-name")).toHaveCount(0);

  await page.clock.fastForward(16_000);
  await expect(state).toContainText("This is taking longer than expected");
  await expect(state.getByRole("button", { name: "Try again" })).toBeVisible();
  await expect(
    state.getByRole("button", { name: "Back to roles" }),
  ).toBeVisible();
});

async function seedHireAsk(
  page: Parameters<typeof installMockBridge>[0],
  input: {
    status: "open" | "resolved";
    outcome?: "approved" | "rejected";
    askResolutionReason?: string;
    hireStatus?: "proposed" | "awaiting_founder" | "approved" | "denied";
    founderPubkey?: string;
    founderApprovalReason?: string;
    relayRole?: "owner" | "admin" | "member" | null;
    askResponseErrors?: string[];
    companyHireActionErrors?: string[];
  },
) {
  const relay = newRelayKey();
  const hireProposal = {
    hireId: HIRE_ASK_HIRE_ID,
    rolePack: {
      personaId: PERSONA_ID,
      title: "Hospitality researcher",
      job: "Hospitality research",
      skills: ["Market research"],
      tools: [{ name: "Web search", risk: "low" as const }],
      workerMenu: ["buzz-agent"],
    },
    displayName: "Mina",
    title: "Hospitality researcher",
    introductionChannelId: GENERAL_CHANNEL_ID,
    runtimeId: "buzz-agent",
    providerId: "openai",
    modelId: "gpt-5.5",
    weeklyAllowance: "8.00",
  };
  const hireHead = finalizeEvent(
    {
      kind: KIND_HIRE_HEAD,
      created_at: Math.floor(Date.now() / 1_000),
      tags: [["d", `company:hire:${HIRE_ASK_HIRE_ID}`]],
      content: JSON.stringify({
        schemaVersion: 1,
        proposal: hireProposal,
        status: input.hireStatus ?? "proposed",
        proposedByPubkey: TEST_IDENTITIES.alice.pubkey,
        sourceAskId: HIRE_ASK_ID,
        sourceAskChannelId: GENERAL_CHANNEL_ID,
        sourceActionEventId: "b".repeat(64),
        ...(input.founderPubkey ? { founderPubkey: input.founderPubkey } : {}),
        ...(input.founderApprovalReason
          ? { founderApprovalReason: input.founderApprovalReason }
          : {}),
      }),
    },
    hexToBytes(relay.privateKeyHex),
  );
  await installMockBridge(page, {
    relaySelf: relay.relaySelf,
    companyAskRelayPrivateKeyHex: relay.privateKeyHex,
    companyHireHeads: [hireHead],
    companyHireRelayPrivateKeyHex: relay.privateKeyHex,
    relayRequiresMembership: true,
    ...(input.relayRole !== undefined ? { relayRole: input.relayRole } : {}),
    ...(input.askResponseErrors
      ? { askResponseErrors: [...input.askResponseErrors] }
      : {}),
    ...(input.companyHireActionErrors
      ? { companyHireActionErrors: [...input.companyHireActionErrors] }
      : {}),
  });
  await page.goto("/#/today");
  await page.waitForFunction(() => {
    const testWindow = window as Window & {
      __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: unknown;
      __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: unknown;
      __BUZZ_E2E_PUBLISH_MOCK_ASK_HEAD__?: unknown;
    };
    return (
      typeof testWindow.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function" &&
      typeof testWindow.__BUZZ_E2E_INVOKE_MOCK_COMMAND__ === "function" &&
      typeof testWindow.__BUZZ_E2E_PUBLISH_MOCK_ASK_HEAD__ === "function"
    );
  });

  const coordinates = await page.evaluate(
    async ({
      askId,
      hireId,
      status,
      outcome,
      askerPubkey,
      hireProposal,
      askResolutionReason,
    }) => {
      type TestWindow = Window & {
        __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
          command: string,
          payload?: unknown,
        ) => Promise<unknown>;
        __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
          channelName: string;
          content: string;
          kind?: number;
          parentEventId?: string | null;
          pubkey?: string;
          extraTags?: string[][];
          id?: string;
        }) => RelayEvent;
        __BUZZ_E2E_PUBLISH_MOCK_ASK_HEAD__?: (input: {
          channelId: string;
          askId: string;
          threadRootEventId: string;
          content: string;
        }) => RelayEvent;
      };
      const testWindow = window as TestWindow;
      const invoke = testWindow.__BUZZ_E2E_INVOKE_MOCK_COMMAND__;
      const emit = testWindow.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
      const publishHead = testWindow.__BUZZ_E2E_PUBLISH_MOCK_ASK_HEAD__;
      if (!invoke || !emit || !publishHead) {
        throw new Error("The mock relay hire ask test seam is unavailable.");
      }
      const identity = (await invoke("get_identity", {})) as {
        pubkey: string;
      };
      const rootId = Array.from(crypto.getRandomValues(new Uint8Array(32)))
        .map((value) => value.toString(16).padStart(2, "0"))
        .join("");
      const root = emit({
        channelName: "general",
        content: "Hire proposal",
        id: rootId,
        pubkey: askerPubkey,
      });
      const channelId = root.tags.find((tag) => tag[0] === "h")?.[1];
      if (!channelId) throw new Error("The seeded hire thread has no channel.");
      const createdAt = new Date().toISOString();
      const ask = {
        schemaVersion: 1,
        askId,
        type: "approval",
        category: "hire",
        title: "Hire a hospitality researcher",
        body: "Proposed by Mina. Scope: hospitality research for Olive Studio. Weekly allowance: USD 8.00.",
        threadRootEventId: root.id,
        addresseePubkey: identity.pubkey,
        decideBy: null,
        subject: { kind: "hire", id: hireId },
        hireProposal,
      };
      const content = JSON.stringify({
        schemaVersion: 1,
        askId,
        status,
        askerPubkey,
        createdAt,
        ask,
        resolution:
          status === "resolved"
            ? {
                outcome,
                ...(askResolutionReason ? { reason: askResolutionReason } : {}),
                resolvedByPubkey: identity.pubkey,
                resolvedAt: createdAt,
                responseEventId: "c".repeat(64),
              }
            : null,
        cancellation: null,
        sourceActionEventId: "b".repeat(64),
      });
      publishHead({ channelId, askId, threadRootEventId: root.id, content });
      emit({
        channelName: "general",
        content: JSON.stringify({
          schemaVersion: 1,
          askId,
          action: "create",
          ask,
        }),
        kind: 47032,
        parentEventId: root.id,
        pubkey: askerPubkey,
        extraTags: [["d", `channel:${channelId}:ask:${askId}`]],
      });
      return { channelId, askId };
    },
    {
      askId: HIRE_ASK_ID,
      hireId: HIRE_ASK_HIRE_ID,
      status: input.status,
      outcome: input.outcome,
      askerPubkey: TEST_IDENTITIES.alice.pubkey,
      hireProposal,
      askResolutionReason: input.askResolutionReason,
    },
  );
  return coordinates;
}

test("hire ask decisions use the reasoned handoff screen", async ({ page }) => {
  const { channelId, askId } = await seedHireAsk(page, { status: "open" });
  await page.goto(`/#/asks/${channelId}/${askId}`);

  const card = page.getByTestId("ask-card");
  await expect(card).toBeVisible();
  await expect(
    card.getByRole("heading", { name: "Decision requested" }),
  ).toBeVisible();
  await expect(card).toContainText(
    "Proposed by Mina. Scope: hospitality research for Olive Studio. Weekly allowance: USD 8.00.",
  );
  await expect(
    card.getByRole("heading", { name: "Decision context" }),
  ).toBeVisible();
  await expect(card).toContainText("npub1mock... · Owner");
  await expect(card.getByRole("button", { name: "Review hire" })).toBeVisible();
  await card.getByRole("button", { name: "Review hire" }).click();
  await expect(page.getByTestId("hire-handoff-form")).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Proposed company position" }),
  ).toBeVisible();
  await expect(page.getByText("Mina", { exact: true })).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Sign off hire" }),
  ).toBeVisible();

  await page.goto(`/#/asks/${channelId}/${askId}`);
  const askCard = page.getByTestId("ask-card");
  await expect(askCard).toBeVisible();
  await askCard.getByRole("button", { name: "Decline" }).click();
  const reason = page.getByTestId("hire-handoff-reason");
  await reason.fill("The current work plan needs another review.");
  await page.getByTestId("hire-handoff-decline").click();
  await expect(
    page.getByText("Request declined", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("The current work plan needs another review."),
  ).toBeVisible();
  await page.goto(`/#/asks/${channelId}/${askId}`);
  const declinedCard = page.getByTestId("ask-card");
  await expect(
    declinedCard.getByRole("heading", { name: "Decision requested" }),
  ).toBeVisible();
  await expect(declinedCard.getByTestId("ask-status")).toHaveText(
    "Request declined",
  );
  await expect(
    declinedCard.getByText("Request declined", { exact: true }),
  ).toBeVisible();
  await expect(declinedCard).toContainText(
    "Proposed by Mina. Scope: hospitality research for Olive Studio. Weekly allowance: USD 8.00.",
  );
  await expect(declinedCard).toContainText(
    /No authority or funding changed\. alice will keep the work paused\./i,
  );
  await expect(
    declinedCard.getByRole("link", { name: "Open conversation" }),
  ).toBeVisible();
});

test("hire ask card keeps its review and offers retry after a failed decline", async ({
  page,
}) => {
  const { channelId, askId } = await seedHireAsk(page, {
    status: "open",
    askResponseErrors: ["Temporary relay write failure"],
  });
  await page.goto(`/#/asks/${channelId}/${askId}`);

  const card = page.getByTestId("ask-card");
  await expect(card).toBeVisible();
  await expect(
    card.getByRole("heading", { name: "Decision requested" }),
  ).toBeVisible();
  await card.getByRole("button", { name: "Decline" }).click();
  const reason = page.getByTestId("hire-handoff-reason");
  await reason.fill("The proposal needs another review.");
  await page.getByTestId("hire-handoff-decline").click();
  await expect(page.getByTestId("hire-handoff-error")).toBeVisible();
  await expect(reason).toHaveValue("The proposal needs another review.");
  await expect(
    page.getByText(
      "Your inputs are kept. Review them or retry without starting again.",
    ),
  ).toBeVisible();
  await page.getByTestId("hire-handoff-decline").click();
  await expect(
    page.getByText("Request declined", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("The proposal needs another review."),
  ).toBeVisible();
  await page.goto(`/#/asks/${channelId}/${askId}`);
  await expect(card.getByTestId("ask-status")).toHaveText("Request declined");
  await expect(
    card.getByText("Request declined", { exact: true }),
  ).toBeVisible();
  await expect(card).toContainText(
    "Proposed by Mina. Scope: hospitality research for Olive Studio. Weekly allowance: USD 8.00.",
  );
});

test("an administrator refers the proposal without hiring the employee", async ({
  page,
}) => {
  const { channelId, askId } = await seedHireAsk(page, {
    status: "open",
    relayRole: "admin",
  });
  await page.goto(`/#/asks/${channelId}/${askId}`);
  await page.getByRole("button", { name: "Review hire" }).click();

  await expect(page.getByText("Approval is a referral")).toBeVisible();
  await page
    .getByTestId("hire-handoff-reason")
    .fill("The role scope is ready for founder review.");
  await page.getByTestId("hire-handoff-approve").click();

  await expect(
    page.getByText("Proposal approved for founder review", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText(
      "No employee has been hired. The founder must review and sign off.",
    ),
  ).toBeVisible();
  await expect(page.getByTestId("hire-open-founder-review")).toBeVisible();
  await page.getByTestId("hire-open-founder-review").click();
  await expect(page.getByTestId("ask-card")).toBeVisible();
  await expect(page.getByTestId("hire-success")).toHaveCount(0);
});

test("founder sign-off is owner-only and retains its reason after a failed write", async ({
  page,
}) => {
  const { channelId, askId } = await seedHireAsk(page, {
    status: "resolved",
    outcome: "approved",
    askResolutionReason: "The proposal is ready for the owner.",
    hireStatus: "awaiting_founder",
    relayRole: "owner",
    companyHireActionErrors: ["Temporary relay write failure"],
  });
  await page.goto(`/#/asks/${channelId}/${askId}`);
  await page.getByTestId("hire-founder-review").click();

  await expect(
    page.getByRole("heading", { name: "Founder sign-off" }),
  ).toBeVisible();
  await expect(
    page.getByText("The proposal is ready for the owner."),
  ).toBeVisible();
  const reason = page.getByTestId("hire-handoff-reason");
  await reason.fill("I approve this role and its recorded scope.");
  await page.getByTestId("hire-handoff-approve").click();
  await expect(page.getByTestId("hire-handoff-error")).toBeVisible();
  await expect(reason).toHaveValue(
    "I approve this role and its recorded scope.",
  );
  await page.getByTestId("hire-handoff-approve").click();

  await expect(
    page.getByRole("heading", { name: "Founder sign-off recorded" }),
  ).toBeVisible();
  await expect(page.getByTestId("hire-open-position")).toBeVisible();
  await page.getByTestId("hire-open-position").click();
  await expect(page.getByTestId("hire-review-form")).toBeVisible();
  await expect(page.getByTestId("hire-founder-confirm")).not.toBeChecked();
  await expect(page.getByTestId("hire-success")).toHaveCount(0);
});

test("a member can view the hire proposal but cannot resolve it", async ({
  page,
}) => {
  const { channelId, askId } = await seedHireAsk(page, {
    status: "open",
    relayRole: "member",
  });
  await page.goto(
    `/#/hire/review?hireId=${HIRE_ASK_HIRE_ID}&channelId=${channelId}&askId=${askId}`,
  );

  await expect(
    page.getByRole("heading", {
      name: "You do not have permission for this action",
    }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Back to the record" }),
  ).toBeVisible();
  await expect(page.getByTestId("hire-handoff-form")).toHaveCount(0);
});

test("resolved hire ask card explains that the requester has the outcome", async ({
  page,
}) => {
  const { channelId, askId } = await seedHireAsk(page, {
    status: "resolved",
    outcome: "approved",
  });
  await page.goto(`/#/asks/${channelId}/${askId}`);

  const card = page.getByTestId("ask-card");
  await expect(
    card.getByRole("heading", { name: "Decision requested" }),
  ).toBeVisible();
  await expect(card.getByTestId("ask-status")).toHaveText("Approved");
  await expect(
    card.getByText("Decision recorded", { exact: true }),
  ).toBeVisible();
  await expect(card).toContainText(
    "The requester has the outcome in the original thread.",
  );
  await expect(card).toContainText(
    "Proposed by Mina. Scope: hospitality research for Olive Studio. Weekly allowance: USD 8.00.",
  );
  await expect(
    card.getByRole("link", { name: "Open conversation" }),
  ).toBeVisible();
});
