import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const SHOTS = "test-results/agent-capability-manifest";
const AGENT_PUBKEY = TEST_IDENTITIES.alice.pubkey;
const REMOTE_AGENT_PUBKEY =
  "a1b2c3d4e5f60718293a4b5c6d7e8f90112233445566778899aabbccddeeff00";
const RELAY_URL = "ws://localhost:3000";

test.use({ viewport: { width: 1440, height: 1400 } });

async function seedCapabilityEvidence(page: import("@playwright/test").Page) {
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_SEED_OBSERVER_EVENTS__ === "function",
    null,
    { timeout: 10_000 },
  );
  const observedAt = Date.now() + 1_000;
  await page.evaluate(
    ({ agentPubkey, timestamp }) => {
      window.__BUZZ_E2E_SEED_OBSERVER_EVENTS__?.({
        agentPubkey,
        events: [
          {
            seq: 1,
            timestamp: new Date(timestamp).toISOString(),
            kind: "agent_initialized",
            agentIndex: 0,
            channelId: null,
            sessionId: null,
            turnId: null,
            payload: {
              initializeResult: {
                protocolVersion: 2,
                agentInfo: { name: "Goose ACP", version: "1.18.0" },
                agentCapabilities: {
                  promptCapabilities: {
                    image: true,
                    audio: false,
                    embeddedContext: true,
                  },
                  outputCapabilities: { image: false, audio: false },
                  tools: [
                    {
                      name: "read_file",
                      source: "filesystem",
                      riskClass: "read",
                      available: true,
                    },
                    {
                      name: "run_command",
                      source: "shell",
                      riskClass: "execute",
                      available: true,
                    },
                  ],
                },
              },
            },
          },
          {
            seq: 2,
            timestamp: new Date(timestamp + 1_000).toISOString(),
            kind: "session_config_captured",
            agentIndex: 0,
            channelId: null,
            sessionId: "session-capabilities",
            turnId: null,
            payload: {
              models: { currentModelId: "claude-opus-4-5" },
              capabilityManifest: {
                modelApplication: {
                  requested: "claude-opus-4-5",
                  applied: true,
                },
                toolSources: [
                  { name: "github", kind: "mcp" },
                  { name: "buzz-dev-mcp", kind: "mcp" },
                ],
                permissionMode: {
                  requested: "bypassPermissions",
                  effective: "perToolAutoDecision",
                  source: "buzzHarness",
                },
              },
            },
          },
          {
            seq: 3,
            timestamp: new Date(timestamp + 2_000).toISOString(),
            kind: "acp_read",
            agentIndex: 0,
            channelId: null,
            sessionId: "session-capabilities",
            turnId: null,
            payload: {
              params: {
                update: {
                  sessionUpdate: "available_commands_update",
                  availableCommands: [
                    { name: "create_plan" },
                    { name: "review_changes" },
                  ],
                },
              },
            },
          },
        ],
      });
    },
    { agentPubkey: AGENT_PUBKEY, timestamp: observedAt },
  );
}

async function openRuntimeManifest(page: import("@playwright/test").Page) {
  await page.getByTestId("channel-agents").click();
  await expect(page.getByTestId("chat-title")).toHaveText("agents");
  const messageRow = page
    .getByTestId("message-row")
    .filter({ has: page.getByText("TARS", { exact: false }) });
  await expect(messageRow.first()).toBeVisible({ timeout: 8_000 });
  await messageRow.first().getByRole("button").first().click();

  const panel = page.getByTestId("user-profile-panel");
  await expect(panel).toBeVisible({ timeout: 10_000 });
  await panel.getByRole("tab", { name: "Runtime" }).click();
  const manifest = panel.getByTestId("agent-capability-manifest");
  await expect(manifest).toBeVisible({ timeout: 10_000 });
  return manifest;
}

test("renders owner-only readiness from catalog, lifecycle, and observer evidence", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: AGENT_PUBKEY,
        name: "TARS",
        status: "running",
        channelNames: ["agents"],
      },
    ],
    managedAgentRuntimes: [
      {
        pubkey: AGENT_PUBKEY,
        relayUrl: RELAY_URL,
        lifecycle: "ready",
      },
    ],
    acpRuntimesCatalog: [
      {
        id: "goose",
        label: "Goose",
        avatar_url: "",
        availability: "available",
        command: "goose",
        binary_path: "/usr/local/bin/goose",
        default_args: ["acp"],
        mcp_command: null,
        install_hint: "",
        install_instructions_url: "https://block.github.io/goose/",
        can_auto_install: true,
        requires_external_cli: true,
        underlying_cli_path: null,
        node_required: false,
        auth_status: { status: "not_applicable" },
        login_hint: null,
        supports_acp_native_config: true,
        supports_acp_model_switching: true,
        mcp_hooks: false,
      },
    ],
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await seedCapabilityEvidence(page);

  const manifest = await openRuntimeManifest(page);
  await expect(
    manifest.getByTestId("agent-capability-overall-status"),
  ).toHaveText("Runtime ready");
  await expect(manifest).toContainText(
    "Local evidence for this owner and machine",
  );
  await expect(manifest).toContainText(
    "not a public safety or reputation claim",
  );
  await expect(
    manifest.getByTestId("agent-readiness-community"),
  ).toHaveAttribute("data-status", "ready");
  await expect(
    manifest.getByTestId("agent-readiness-observer"),
  ).toHaveAttribute("data-status", "ready");
  await expect(
    manifest.getByTestId("agent-capability-permission-mode"),
  ).toContainText("perToolAutoDecision");
  await expect(
    manifest.getByTestId("agent-capability-permission-mode"),
  ).toContainText("Source: Buzz harness");
  await expect(manifest).toContainText("run_command");
  await expect(manifest).toContainText("buzz-dev-mcp");
  await waitForAnimations(page);
  await manifest.getByTestId("agent-capability-readiness-evidence").screenshot({
    path: `${SHOTS}/01-readiness-and-runtime-evidence.png`,
  });
  await waitForAnimations(page);
  await manifest
    .getByTestId("agent-capability-delegation-evidence")
    .screenshot({
      path: `${SHOTS}/02-capabilities-permissions-and-tools.png`,
    });
  await waitForAnimations(page);
  await manifest.screenshot({ path: `${SHOTS}/03-complete-manifest.png` });
});

test("does not expose the manifest to a non-owner", async ({ page }) => {
  await installMockBridge(page, {
    relayAgents: [
      {
        pubkey: REMOTE_AGENT_PUBKEY,
        name: "nadia",
        agentType: "goose",
        channelNames: ["agents"],
      },
    ],
    searchProfiles: [
      {
        pubkey: REMOTE_AGENT_PUBKEY,
        displayName: "nadia",
        ownerPubkey: TEST_IDENTITIES.bob.pubkey,
        isAgent: true,
      },
    ],
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("channel-agents").click();
  const messageRow = page.getByTestId("message-row").filter({
    has: page.getByText("Indexing remotely for my owner."),
  });
  await expect(messageRow.first()).toBeVisible({ timeout: 5_000 });
  await messageRow.first().getByRole("button").first().click();

  const panel = page.getByTestId("user-profile-panel");
  await expect(panel).toBeVisible({ timeout: 10_000 });
  await expect(panel.getByRole("tab", { name: "Runtime" })).toHaveCount(0);
  await expect(panel.getByTestId("agent-capability-manifest")).toHaveCount(0);
});
