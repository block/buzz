import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";

// Layer 3 of the mesh e2e: the desktop UI contract for mesh-compute, driven
// through the real UI + the (mocked) Tauri mesh commands. Asserts the bridge
// CALL ORDER, not just labels — in particular the prepare-before-spawn
// invariant and the membership-denial copy.

type E2eWindow = Window & {
  __BUZZ_E2E__?: { mock?: { meshReporterPubkey?: string } };
  __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: unknown;
  __TAURI_INTERNALS__?: { invoke?: unknown };
  __BUZZ_E2E_COMMANDS__?: string[];
  __BUZZ_E2E_SIGNED_EVENTS__?: Array<{
    content: string;
    kind: number;
    tags: string[][];
  }>;
  __BUZZ_E2E_SET_MESH__?: (mesh: {
    admitted?: boolean;
    models?: Array<{ id: string; name: string | null }>;
    denyReason?: string;
  }) => void;
};

async function waitForInvokeBridge(page: import("@playwright/test").Page) {
  await page.waitForFunction(() => {
    const w = window as E2eWindow;
    return (
      typeof w.__BUZZ_E2E_INVOKE_MOCK_COMMAND__ === "function" ||
      typeof w.__TAURI_INTERNALS__?.invoke === "function"
    );
  }, null);
}

async function gotoApp(page: import("@playwright/test").Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await waitForInvokeBridge(page);
  await expect(page.getByTestId("open-agents-view")).toBeVisible({
    timeout: 10_000,
  });
}

/** Ordered command names the bridge recorded so far. */
async function commands(page: import("@playwright/test").Page) {
  return page.evaluate(() => (window as E2eWindow).__BUZZ_E2E_COMMANDS__ ?? []);
}

/** Signed event templates the bridge recorded so far. */
async function signedEvents(page: import("@playwright/test").Page) {
  return page.evaluate(
    () => (window as E2eWindow).__BUZZ_E2E_SIGNED_EVENTS__ ?? [],
  );
}

async function setMesh(
  page: import("@playwright/test").Page,
  mesh: { admitted?: boolean; denyReason?: string },
) {
  await page.evaluate((m) => {
    (window as E2eWindow).__BUZZ_E2E_SET_MESH__?.(m);
  }, mesh);
}

async function triggerManagedAgentPrimaryAction(
  page: import("@playwright/test").Page,
  agentName: string,
) {
  // Agent lifecycle actions moved from the old per-row dropdown into the
  // profile sidebar (PR #1200): the Agents-page surfaces each agent as an
  // identity card that opens the profile panel on click, where a single
  // primary-action button toggles Stop (when running/deployed) / Start (when
  // stopped). Open the panel for this agent if it isn't already showing it,
  // then click that toggle. The card is looked up by its aria-label because
  // persona-linked agents (the definition_start flow) render under the
  // persona group card, which has no managed-agent-<pubkey> testid.
  const panel = page.getByTestId("user-profile-panel");
  const primaryAction = panel.getByTestId("user-profile-agent-primary-action");
  if (!(await primaryAction.isVisible().catch(() => false))) {
    await page
      .getByRole("button", { name: `${agentName} agent profile` })
      .first()
      .click();
    await expect(panel).toBeVisible();
  }
  await expect(primaryAction).toBeEnabled();
  await primaryAction.click();
}

async function openNewAgentMenu(page: import("@playwright/test").Page) {
  await page.getByTestId("new-agent-card").click();
}

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

test("Share compute starts and stops a serve node", async ({ page }) => {
  await gotoApp(page);
  await openSettings(page, "compute");

  const card = page.getByTestId("settings-mesh-share-compute");
  await expect(card).toBeVisible({ timeout: 10_000 });

  // A model ref is required before the machine can be shared (the toggle IS
  // the start/stop control).
  await page
    .getByTestId("mesh-share-compute-model")
    .fill("hf://demo/SmolLM2-135M-Instruct-GGUF:Q4_K_M");

  const toggle = page.getByTestId("mesh-share-compute-toggle");
  await expect(toggle).toBeEnabled({ timeout: 10_000 });

  // Toggle on -> starts a serve node.
  await toggle.click();
  await expect
    .poll(async () => await commands(page))
    .toContain("mesh_start_node");

  // Status polling flips the toggle into the "on" state…
  await expect(toggle).toBeChecked({ timeout: 10_000 });

  // …toggle off -> stops the serve node.
  await toggle.click();
  await expect
    .poll(async () => await commands(page))
    .toContain("mesh_stop_node");
});

test("Share compute model draft persists across reload", async ({ page }) => {
  await gotoApp(page);
  await openSettings(page, "compute");

  const model = page.getByTestId("mesh-share-compute-model");
  await expect(model).toBeVisible({ timeout: 10_000 });
  await model.fill("unsloth/Qwen3.6-35B-A3B-GGUF@main:UD-Q4_K_S");

  await page.getByText("Advanced").click();
  await page.getByTestId("mesh-share-compute-vram").fill("42");

  await page.reload({ waitUntil: "domcontentloaded" });
  // Settings now lives in the history stack (/settings?section=…), so a reload
  // restores the open section straight from the URL — no need to navigate back
  // through the app shell.
  await expect(page.getByTestId("settings-view")).toBeVisible({
    timeout: 10_000,
  });

  await expect(page.getByTestId("mesh-share-compute-model")).toHaveValue(
    "unsloth/Qwen3.6-35B-A3B-GGUF@main:UD-Q4_K_S",
  );
  await page.getByText("Advanced").click();
  await expect(page.getByTestId("mesh-share-compute-vram")).toHaveValue("42");
});

test("Run-on-relay-mesh ensures the client node BEFORE spawning the agent", async ({
  page,
}) => {
  await gotoApp(page);
  await page.getByTestId("open-agents-view").click();
  await openNewAgentMenu(page);
  await page.getByRole("menuitem", { name: /^New agent$/ }).click();

  // Member is admitted -> the relay-mesh toggle becomes enabled (availability
  // resolves to available). Wait for that before driving the flow.
  await page.locator("#persona-display-name").fill("Mesh Agent");

  const toggle = page.getByTestId("agent-relay-mesh-toggle");
  await expect(toggle).toBeEnabled({ timeout: 10_000 });
  await toggle.click();
  await page
    .getByTestId("agent-relay-mesh-model")
    .selectOption({ label: "SmolLM2 135M — Mock desktop" });

  await expect(page.getByTestId("persona-dialog-submit")).toBeEnabled({
    timeout: 10_000,
  });

  // Snapshot the command log length right before the click so we assert the
  // ensure→spawn ordering WITHIN this user action's fresh slice, not merely
  // that ensure appeared somewhere earlier (e.g. an availability probe).
  const before = (await commands(page)).length;
  await page.getByTestId("persona-dialog-submit").click();

  await expect
    .poll(async () => (await commands(page)).slice(before))
    .toContain("create_managed_agent");

  // The invariant: within the Create action, prepare runs BEFORE create.
  const slice = (await commands(page)).slice(before);
  const prepareIdx = slice.indexOf("mesh_prepare_relay_mesh_client");
  const createIdx = slice.indexOf("create_managed_agent");
  expect(
    prepareIdx,
    "prepare must occur in the Create action",
  ).toBeGreaterThanOrEqual(0);
  expect(prepareIdx).toBeLessThan(createIdx);
});

test("Run-on-relay-mesh skips connect signaling for own serve target", async ({
  page,
}) => {
  await gotoApp(page);
  await page.getByTestId("open-agents-view").click();
  await openNewAgentMenu(page);
  await page.getByRole("menuitem", { name: /^New agent$/ }).click();
  await page.locator("#persona-display-name").fill("Own Mesh Agent");

  const toggle = page.getByTestId("agent-relay-mesh-toggle");
  await expect(toggle).toBeEnabled({ timeout: 10_000 });
  await toggle.click();
  await page
    .getByTestId("agent-relay-mesh-model")
    .selectOption({ label: "SmolLM2 135M — Mock desktop" });

  const before = (await commands(page)).length;
  await page.getByTestId("persona-dialog-submit").click();
  await expect
    .poll(async () => (await commands(page)).slice(before))
    .toContain("create_managed_agent");

  const slice = (await commands(page)).slice(before);
  expect(slice).toContain("mesh_prepare_relay_mesh_client");
  expect(
    (await signedEvents(page)).filter((event) => event.kind === 24621),
  ).toHaveLength(0);
});

test("Run-on-relay-mesh canonicalizes the mesh connect #p target", async ({
  page,
}) => {
  await page.addInitScript(() => {
    const w = window as E2eWindow;
    w.__BUZZ_E2E__ = {
      ...(w.__BUZZ_E2E__ ?? {}),
      mock: {
        ...(w.__BUZZ_E2E__?.mock ?? {}),
        meshReporterPubkey:
          "  CAFEBABECAFEBABECAFEBABECAFEBABECAFEBABECAFEBABECAFEBABECAFEBABE  ",
      },
    };
  });
  await gotoApp(page);
  await page.getByTestId("open-agents-view").click();
  await openNewAgentMenu(page);
  await page.getByRole("menuitem", { name: /^New agent$/ }).click();
  await page.locator("#persona-display-name").fill("Mesh Agent");

  const toggle = page.getByTestId("agent-relay-mesh-toggle");
  await expect(toggle).toBeEnabled({ timeout: 10_000 });
  await toggle.click();
  await page
    .getByTestId("agent-relay-mesh-model")
    .selectOption({ label: "SmolLM2 135M — Mock desktop" });

  await page.getByTestId("persona-dialog-submit").click();
  await expect
    .poll(async () =>
      (await signedEvents(page)).find((event) => event.kind === 24621),
    )
    .toMatchObject({
      tags: [
        [
          "p",
          "cafebabecafebabecafebabecafebabecafebabecafebabecafebabecafebabe",
        ],
      ],
    });
  await expect
    .poll(async () => await commands(page))
    .toContain("create_managed_agent");
});

test("a non-member cannot enable relay-mesh — membership is the gate", async ({
  page,
}) => {
  await gotoApp(page);
  // Non-member: relay membership is the only factor, so availability reports
  // unavailable. There is no manual auth action that would change this.
  await setMesh(page, { admitted: false, denyReason: "not a relay member" });

  await page.getByTestId("open-agents-view").click();
  await openNewAgentMenu(page);
  await page.getByRole("menuitem", { name: /^New agent$/ }).click();

  // The relay-mesh toggle stays disabled — a non-member cannot even opt into
  // running on the mesh, let alone spawn an agent against it.
  const toggle = page.getByTestId("agent-relay-mesh-toggle");
  await expect(toggle).toBeDisabled();

  // The flow never reaches an ensure-or-spawn, because membership gates the
  // entry point itself. Sanity-check we never created an agent on the mesh.
  const seq = await commands(page);
  expect(seq).not.toContain("create_managed_agent");
});

test("saved relay-mesh agents restart via the backend serve-target preflight", async ({
  page,
}) => {
  await gotoApp(page);
  await page.getByTestId("open-agents-view").click();
  await openNewAgentMenu(page);
  await page.getByRole("menuitem", { name: /^New agent$/ }).click();
  await page.locator("#persona-display-name").fill("Saved relay mesh agent");

  const toggle = page.getByTestId("agent-relay-mesh-toggle");
  await expect(toggle).toBeEnabled({ timeout: 10_000 });
  await toggle.click();
  await page
    .getByTestId("agent-relay-mesh-model")
    .selectOption({ label: "SmolLM2 135M — Mock desktop" });

  await page.getByTestId("persona-dialog-submit").click();
  await expect
    .poll(async () => await commands(page))
    .toContain("create_managed_agent");

  const agents = await page.evaluate(async () => {
    const w = window as E2eWindow & {
      __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
        command: string,
        payload?: Record<string, unknown>,
      ) => Promise<Array<{ name: string; pubkey: string }>>;
    };
    const invoke = w.__BUZZ_E2E_INVOKE_MOCK_COMMAND__;
    if (!invoke) throw new Error("Mock invoke bridge is unavailable.");
    return invoke("list_managed_agents");
  });
  const pubkey = agents.find(
    (agent) => agent.name === "Saved relay mesh agent",
  )?.pubkey;
  expect(pubkey).toBeTruthy();

  // Dismiss the reveal dialog BEFORE asserting the card: the modal makes the
  // rest of the page aria-hidden, so role-based queries can't see it yet.
  await page.getByRole("button", { name: "Done" }).click();
  await expect(page.getByRole("dialog", { name: "Agent created" })).toHaveCount(
    0,
  );

  // definition_start mints the instance WITH a personaId, so it files under
  // the persona group card (persona-agent-row-<personaId>), not the
  // standalone managed-agent-<pubkey> card. Locate the card by the runtime
  // control keyed on the instance pubkey and assert the agent's NAME reaches
  // that same card (the profile-open button is an empty aria-label overlay,
  // so the text check must target the card container).
  const card = page
    .locator('[data-testid^="persona-agent-row-"]')
    .filter({ has: page.getByTestId(`agent-runtime-active-${pubkey}`) });
  await expect(card).toContainText("Saved relay mesh agent");

  await triggerManagedAgentPrimaryAction(page, "Saved relay mesh agent");
  await expect
    .poll(async () => await commands(page))
    .toContain("stop_managed_agent");
  await expect(page.getByTestId(`agent-runtime-start-${pubkey}`)).toBeVisible();

  // With a live serve target for the model, manual restart goes through:
  // the backend preflight re-resolves the target and the agent starts.
  await triggerManagedAgentPrimaryAction(page, "Saved relay mesh agent");
  await expect
    .poll(async () => await commands(page))
    .toContain("start_managed_agent");
  await expect(
    page.getByTestId(`agent-runtime-active-${pubkey}`),
  ).toBeVisible();

  await triggerManagedAgentPrimaryAction(page, "Saved relay mesh agent");
  await expect(page.getByTestId(`agent-runtime-start-${pubkey}`)).toBeVisible();

  // Without a live serve target, the backend preflight rejects the start
  // with an actionable error, surfaced as a toast; the agent stays stopped.
  await setMesh(page, { models: [] });
  await triggerManagedAgentPrimaryAction(page, "Saved relay mesh agent");

  await expect(
    page
      .locator("[data-sonner-toast]")
      .filter({ hasText: "no live serve target is available" }),
  ).toBeVisible();
  await expect(page.getByTestId(`agent-runtime-start-${pubkey}`)).toBeVisible();

  await expect(
    page.evaluate(async (agentPubkey) => {
      const invoke = (window as E2eWindow).__BUZZ_E2E_INVOKE_MOCK_COMMAND__ as
        | ((
            command: string,
            payload?: Record<string, unknown>,
          ) => Promise<unknown>)
        | undefined;
      if (!invoke) throw new Error("Mock invoke bridge is unavailable.");
      try {
        await invoke("start_managed_agent", { pubkey: agentPubkey });
        return "started";
      } catch (err) {
        return err instanceof Error ? err.message : String(err);
      }
    }, pubkey),
  ).resolves.toContain("no live serve target is available");
  await expect(page.getByTestId(`agent-runtime-start-${pubkey}`)).toBeVisible();
});
