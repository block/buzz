import { test, expect } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";

const SHOTS = "test-results/byoh-after";

/**
 * AFTER screenshots for the consolidated Harnesses surface — same catalog
 * fixture as byoh-before-screenshots.spec.ts so the before/after pair is a
 * true apples-to-apples comparison.
 */
const CATALOG = [
  {
    id: "buzz-agent",
    label: "Buzz Agent",
    avatar_url: "",
    availability: "available",
    command: "buzz-agent",
    binary_path: "/usr/local/bin/buzz-agent",
    default_args: [],
    mcp_command: "buzz-dev-mcp",
    install_hint: "",
    install_instructions_url: "https://github.com/block/buzz",
    can_auto_install: false,
    underlying_cli_path: null,
    node_required: false,
    auth_status: { status: "not_applicable" },
  },
  {
    id: "claude",
    label: "Claude Code",
    avatar_url: "",
    availability: "available",
    command: "claude-agent-acp",
    binary_path: "/usr/local/bin/claude-agent-acp",
    default_args: [],
    mcp_command: null,
    install_hint: "",
    install_instructions_url:
      "https://github.com/agentclientprotocol/claude-agent-acp",
    can_auto_install: true,
    underlying_cli_path: "/usr/local/bin/claude",
    node_required: false,
    auth_status: { status: "logged_in" },
  },
  {
    id: "codex",
    label: "Codex",
    avatar_url: "",
    availability: "adapter_missing",
    command: null,
    binary_path: null,
    default_args: [],
    mcp_command: null,
    install_hint: "Install the Codex ACP adapter via npm.",
    install_instructions_url:
      "https://github.com/agentclientprotocol/codex-acp",
    can_auto_install: true,
    underlying_cli_path: "/usr/local/bin/codex",
    node_required: false,
    auth_status: { status: "unknown" },
  },
  {
    id: "cursor",
    label: "Cursor",
    avatar_url: "",
    availability: "available",
    command: "cursor-agent",
    binary_path: "/usr/local/bin/cursor-agent",
    default_args: ["acp"],
    mcp_command: null,
    install_hint:
      "Buzz talks to Cursor through the cursor-agent CLI's ACP mode.",
    install_instructions_url: "https://cursor.com/downloads",
    can_auto_install: false,
    underlying_cli_path: null,
    node_required: false,
    auth_status: { status: "not_applicable" },
    source: "preset",
  },
  {
    id: "omp",
    label: "Oh My Pi",
    avatar_url: "",
    availability: "not_installed",
    command: null,
    binary_path: null,
    default_args: ["acp"],
    mcp_command: null,
    install_hint:
      "Buzz talks to Oh My Pi through its CLI's ACP mode (omp acp).",
    install_instructions_url: "https://github.com/can1357/oh-my-pi",
    can_auto_install: false,
    underlying_cli_path: null,
    node_required: false,
    auth_status: { status: "not_applicable" },
    source: "preset",
  },
  {
    id: "opencode",
    label: "OpenCode",
    avatar_url: "",
    availability: "not_installed",
    command: null,
    binary_path: null,
    default_args: ["acp"],
    mcp_command: null,
    install_hint:
      "Buzz talks to OpenCode through its CLI's ACP mode (opencode acp).",
    install_instructions_url: "https://opencode.ai/docs",
    can_auto_install: false,
    underlying_cli_path: null,
    node_required: false,
    auth_status: { status: "not_applicable" },
    source: "preset",
  },
  {
    id: "my-custom",
    label: "My Custom Harness",
    avatar_url: "",
    availability: "available",
    command: "my-acp",
    binary_path: "/usr/local/bin/my-acp",
    default_args: [],
    mcp_command: null,
    install_hint: "",
    install_instructions_url: "",
    can_auto_install: false,
    underlying_cli_path: null,
    node_required: false,
    auth_status: { status: "not_applicable" },
    source: "custom",
  },
];

test("after: consolidated harnesses panel + catalog dialog", async ({
  page,
}) => {
  test.setTimeout(60_000);
  await page.setViewportSize({ width: 1280, height: 2400 });
  await installMockBridge(page, { acpRuntimesCatalog: CATALOG });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await openSettings(page, "agents");
  await expect(page.getByTestId("settings-harnesses")).toBeVisible({
    timeout: 10_000,
  });
  await page.waitForTimeout(700);

  // 1. The consolidated panel — ready rows only, no inert controls.
  await page.getByTestId("settings-harnesses").screenshot({
    path: `${SHOTS}/after-harnesses-panel.png`,
  });

  // 2. Add-harnesses catalog — needs-setup entry auto-selected in detail.
  await page.getByTestId("harness-add-button").click();
  await expect(page.getByTestId("harness-catalog-dialog")).toBeVisible();
  await page.waitForTimeout(500);
  await page.getByTestId("harness-catalog-dialog").screenshot({
    path: `${SHOTS}/after-catalog-needs-setup.png`,
  });

  // 3. Ready entry detail (Ready state, no install action). Ready entries
  // sit in the "Installed" accordion, collapsed by default — expand it.
  await page.getByTestId("harness-catalog-section-installed").click();
  await page.getByTestId("harness-catalog-list-item-claude").click();
  await page.waitForTimeout(300);
  await page.getByTestId("harness-catalog-dialog").screenshot({
    path: `${SHOTS}/after-catalog-ready-detail.png`,
  });

  // 4. Custom harness form — all fields inline.
  await page.getByTestId("harness-catalog-list-item-custom").click();
  await expect(page.getByTestId("custom-harness-form")).toBeVisible();
  await page.waitForTimeout(300);
  await page.getByTestId("harness-catalog-dialog").screenshot({
    path: `${SHOTS}/after-catalog-custom-form.png`,
  });
});
