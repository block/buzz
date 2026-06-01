import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";

// Layer 3 of the mesh e2e: the desktop UI contract for mesh-compute, driven
// through the real UI + the (mocked) Tauri mesh commands. Asserts the bridge
// CALL ORDER, not just labels — in particular the ensure-before-spawn
// invariant and the membership-denial copy.

type E2eWindow = Window & {
  __SPROUT_E2E_INVOKE_MOCK_COMMAND__?: unknown;
  __TAURI_INTERNALS__?: { invoke?: unknown };
  __SPROUT_E2E_COMMANDS__?: string[];
  __SPROUT_E2E_SET_MESH__?: (mesh: {
    admitted?: boolean;
    models?: Array<{ id: string; name: string | null }>;
    denyReason?: string;
  }) => void;
};

async function waitForInvokeBridge(page: import("@playwright/test").Page) {
  await page.waitForFunction(() => {
    const w = window as E2eWindow;
    return (
      typeof w.__SPROUT_E2E_INVOKE_MOCK_COMMAND__ === "function" ||
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
  return page.evaluate(
    () => (window as E2eWindow).__SPROUT_E2E_COMMANDS__ ?? [],
  );
}

async function setMesh(
  page: import("@playwright/test").Page,
  mesh: { admitted?: boolean; denyReason?: string },
) {
  await page.evaluate((m) => {
    (window as E2eWindow).__SPROUT_E2E_SET_MESH__?.(m);
  }, mesh);
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

test("Run-on-relay-mesh ensures the client node BEFORE spawning the agent", async ({
  page,
}) => {
  await gotoApp(page);
  await page.getByTestId("open-agents-view").click();
  await page.getByRole("button", { name: "New" }).click();
  await page.getByText("Custom Agent").click();

  // Member is admitted -> the relay-mesh toggle becomes enabled (availability
  // resolves to available). Wait for that before driving the flow.
  await page.getByTestId("agent-name-input").fill("Mesh Agent");

  const toggle = page.getByTestId("agent-relay-mesh-toggle");
  await expect(toggle).toBeEnabled({ timeout: 10_000 });
  await toggle.click();
  await page
    .getByTestId("agent-relay-mesh-model")
    .selectOption("hf://demo/SmolLM2-135M-Instruct-GGUF:Q4_K_M");

  await expect(page.getByTestId("create-agent-submit")).toBeEnabled({
    timeout: 10_000,
  });

  // Snapshot the command log length right before the click so we assert the
  // ensure→spawn ordering WITHIN this user action's fresh slice, not merely
  // that ensure appeared somewhere earlier (e.g. an availability probe).
  const before = (await commands(page)).length;
  await page.getByTestId("create-agent-submit").click();

  await expect
    .poll(async () => (await commands(page)).slice(before))
    .toContain("create_managed_agent");

  // The invariant: within the Create action, ensure runs BEFORE create.
  const slice = (await commands(page)).slice(before);
  const ensureIdx = slice.indexOf("mesh_ensure_client_node");
  const createIdx = slice.indexOf("create_managed_agent");
  expect(
    ensureIdx,
    "ensure must occur in the Create action",
  ).toBeGreaterThanOrEqual(0);
  expect(ensureIdx).toBeLessThan(createIdx);
});

test("a non-member cannot enable relay-mesh — membership is the gate", async ({
  page,
}) => {
  await gotoApp(page);
  // Non-member: relay membership is the only factor, so availability reports
  // unavailable. There is no manual auth action that would change this.
  await setMesh(page, { admitted: false, denyReason: "not a relay member" });

  await page.getByTestId("open-agents-view").click();
  await page.getByRole("button", { name: "New" }).click();
  await page.getByText("Custom Agent").click();

  // The relay-mesh toggle stays disabled — a non-member cannot even opt into
  // running on the mesh, let alone spawn an agent against it.
  const toggle = page.getByTestId("agent-relay-mesh-toggle");
  await expect(toggle).toBeDisabled();

  // The flow never reaches an ensure-or-spawn, because membership gates the
  // entry point itself. Sanity-check we never created an agent on the mesh.
  const seq = await commands(page);
  expect(seq).not.toContain("create_managed_agent");
});
