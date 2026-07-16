import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";
import { openSettings } from "../helpers/settings";

const SHOTS = "test-results/screenshots-doctor";

// ── Shared catalog fixture data ───────────────────────────────────────────────

/**
 * A goose runtime that is available and needs no auth step — used as a neutral
 * backdrop so the Doctor panel has realistic content beyond the row under test.
 */
const GOOSE_AVAILABLE = {
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
  can_auto_install: false,
  underlying_cli_path: null,
  node_required: false,
  auth_status: { status: "not_applicable" },
};

/** buzz-agent is always available and has no auth step. */
const BUZZ_AGENT_AVAILABLE = {
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
};

/**
 * Claude available and logged in — used as a neutral entry when claude is not
 * the runtime under test, and as the base for the auth states being tested.
 */
const CLAUDE_AVAILABLE_LOGGED_IN = {
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
};

/**
 * Codex not-installed base — tweak `availability`, `auth_status`, and
 * `node_required` in each test as needed.
 */
const CODEX_NOT_INSTALLED = {
  id: "codex",
  label: "Codex",
  avatar_url: "",
  availability: "not_installed",
  command: null,
  binary_path: null,
  default_args: [],
  mcp_command: null,
  install_hint: "Install the Codex CLI, then install the ACP adapter via npm.",
  install_instructions_url: "https://github.com/zed-industries/codex-acp",
  can_auto_install: true,
  underlying_cli_path: null,
  node_required: false,
  auth_status: { status: "unknown" },
};

// ── Tests ─────────────────────────────────────────────────────────────────────

test.describe("Doctor panel state screenshots", () => {
  test.use({ viewport: { width: 1280, height: 720 } });

  test.beforeEach(async ({ page }) => {
    page.on("pageerror", (err) => {
      console.error(
        "PAGE ERROR:",
        err.message,
        err.stack?.split("\n").slice(0, 3).join("\n"),
      );
    });
    page.on("console", (msg) => {
      if (msg.type() === "error") {
        console.error("CONSOLE ERROR:", msg.text().slice(0, 300));
      }
    });
  });

  /**
   * 01 — available runtime that passed the auth probe: green "Authenticated"
   * badge appears below the binary path.
   */
  test("01-auth-logged-in", async ({ page }) => {
    await installMockBridge(page, {
      acpRuntimesCatalog: [
        GOOSE_AVAILABLE,
        CLAUDE_AVAILABLE_LOGGED_IN,
        CODEX_NOT_INSTALLED,
        BUZZ_AGENT_AVAILABLE,
      ],
    });

    await page.goto("/", { waitUntil: "domcontentloaded" });
    await openSettings(page, "doctor");

    const row = page.getByTestId("doctor-runtime-claude");
    await expect(row).toBeVisible({ timeout: 10_000 });
    await expect(row).toContainText("Authenticated");

    await row.scrollIntoViewIfNeeded();
    await waitForAnimations(page);
    await row.screenshot({ path: `${SHOTS}/01-auth-logged-in.png` });
  });

  /**
   * 02 — available runtime that failed the auth probe: amber "Not
   * authenticated" badge + login hint shown below the binary path.
   */
  test("02-auth-logged-out", async ({ page }) => {
    await installMockBridge(page, {
      acpRuntimesCatalog: [
        GOOSE_AVAILABLE,
        CLAUDE_AVAILABLE_LOGGED_IN,
        {
          ...CODEX_NOT_INSTALLED,
          availability: "available",
          command: "codex-acp",
          binary_path: "/usr/local/bin/codex-acp",
          underlying_cli_path: "/usr/local/bin/codex",
          auth_status: { status: "logged_out" },
          login_hint: "Run `codex login` to authenticate.",
        },
        BUZZ_AGENT_AVAILABLE,
      ],
    });

    await page.goto("/", { waitUntil: "domcontentloaded" });
    await openSettings(page, "doctor");

    const row = page.getByTestId("doctor-runtime-codex");
    await expect(row).toBeVisible({ timeout: 10_000 });
    await expect(row).toContainText("Not authenticated");
    await expect(row).toContainText("Run `codex login` to authenticate.");

    await row.scrollIntoViewIfNeeded();
    await waitForAnimations(page);
    await row.screenshot({ path: `${SHOTS}/02-auth-logged-out.png` });
  });

  /**
   * 03 — available runtime whose CLI has a config-parse error: red "Config
   * error" badge + diagnostic excerpt shown below the binary path.
   */
  test("03-auth-config-error", async ({ page }) => {
    const diagnostic =
      "error loading configuration: ~/.claude/settings.json: unknown key foo";
    await installMockBridge(page, {
      acpRuntimesCatalog: [
        GOOSE_AVAILABLE,
        {
          ...CLAUDE_AVAILABLE_LOGGED_IN,
          auth_status: { status: "config_invalid", diagnostic },
          login_hint: "Run the Claude CLI to complete authentication.",
        },
        CODEX_NOT_INSTALLED,
        BUZZ_AGENT_AVAILABLE,
      ],
    });

    await page.goto("/", { waitUntil: "domcontentloaded" });
    await openSettings(page, "doctor");

    const row = page.getByTestId("doctor-runtime-claude");
    await expect(row).toBeVisible({ timeout: 10_000 });
    await expect(row).toContainText("Config error");
    await expect(row).toContainText("error loading configuration");

    await row.scrollIntoViewIfNeeded();
    await waitForAnimations(page);
    await row.screenshot({ path: `${SHOTS}/03-auth-config-error.png` });
  });

  /**
   * 04 — adapter_missing runtime with node_required: true: the amber "Node.js
   * is required…" callout replaces the Install button so the user cannot
   * inadvertently trigger a doomed npm install.
   */
  test("04-node-required", async ({ page }) => {
    await installMockBridge(page, {
      acpRuntimesCatalog: [
        GOOSE_AVAILABLE,
        CLAUDE_AVAILABLE_LOGGED_IN,
        {
          ...CODEX_NOT_INSTALLED,
          availability: "adapter_missing",
          underlying_cli_path: "/usr/local/bin/codex",
          node_required: true,
          install_hint:
            "Install the Codex ACP adapter: npm install -g @zed-industries/codex-acp",
        },
        BUZZ_AGENT_AVAILABLE,
      ],
    });

    await page.goto("/", { waitUntil: "domcontentloaded" });
    await openSettings(page, "doctor");

    const row = page.getByTestId("doctor-runtime-codex");
    await expect(row).toBeVisible({ timeout: 10_000 });
    await expect(row).toContainText("Node.js is required");
    // Exact-name match so "Install Node.js" (inside the callout) is not counted.
    await expect(
      row.getByRole("button", { name: "Install", exact: true }),
    ).toHaveCount(0);

    await row.scrollIntoViewIfNeeded();
    await waitForAnimations(page);
    await row.screenshot({ path: `${SHOTS}/04-node-required.png` });
  });

  /**
   * 05 — a failed install renders a "Retry" button; clicking Retry succeeds.
   *
   * The mock is configured with a two-call sequence:
   *   call 1 → failure (E404)
   *   call 2 → success
   * This exercises the full Retry UX path: fail state → click Retry →
   * spinner → success banner.
   */
  test("05-retry-after-failure", async ({ page }) => {
    await installMockBridge(page, {
      acpRuntimesCatalog: [
        GOOSE_AVAILABLE,
        CLAUDE_AVAILABLE_LOGGED_IN,
        {
          ...CODEX_NOT_INSTALLED,
          can_auto_install: true,
          node_required: false,
        },
        BUZZ_AGENT_AVAILABLE,
      ],
      installAcpRuntimeResults: [
        {
          success: false,
          steps: [
            {
              step: "adapter",
              command: "npm install -g @zed-industries/codex-acp",
              success: false,
              stdout: "",
              stderr: "npm ERR! code E404\nnpm ERR! 404 Not Found",
              exit_code: 1,
            },
          ],
        },
        {
          success: true,
          steps: [
            {
              step: "adapter",
              command: "npm install -g @zed-industries/codex-acp",
              success: true,
              stdout: "added 1 package",
              stderr: "",
              exit_code: 0,
            },
          ],
        },
      ],
    });

    await page.goto("/", { waitUntil: "domcontentloaded" });
    await openSettings(page, "doctor");

    const row = page.getByTestId("doctor-runtime-codex");
    await expect(row).toBeVisible({ timeout: 10_000 });

    // Trigger the first install — the mock returns a failure.
    const installBtn = row.getByRole("button", { name: "Install Codex" });
    await expect(installBtn).toBeVisible({ timeout: 5_000 });
    await installBtn.click();

    // After failure: Retry button appears and the error message is visible.
    const retryBtn = row.getByRole("button", { name: "Retry Install Codex" });
    await expect(retryBtn).toBeVisible({ timeout: 5_000 });
    await expect(row).toContainText("Step");
    await expect(row).toContainText("failed");

    await row.scrollIntoViewIfNeeded();
    await waitForAnimations(page);
    await row.screenshot({ path: `${SHOTS}/05-retry-after-failure.png` });

    // Click Retry — the mock returns success on the second call.
    await retryBtn.click();

    // Error paragraph must disappear and per-runtime spinner must appear,
    // then the success banner must render.
    await expect(row).not.toContainText("failed", { timeout: 5_000 });
    await expect(
      row.getByText("Codex installed. Checking for sign-in options..."),
    ).toBeVisible({
      timeout: 10_000,
    });

    await row.scrollIntoViewIfNeeded();
    await waitForAnimations(page);
    await row.screenshot({ path: `${SHOTS}/05-retry-success.png` });
  });

  /**
   * 06 — logged-out runtime with adapter-advertised auth methods: Doctor shows
   * adapter-provided labels/descriptions and clicking one launches the
   * vendor-owned flow through the mocked connect command.
   */
  test("06-connect-account-methods", async ({ page }) => {
    await installMockBridge(page, {
      acpRuntimesCatalog: [
        GOOSE_AVAILABLE,
        CLAUDE_AVAILABLE_LOGGED_IN,
        {
          ...CODEX_NOT_INSTALLED,
          availability: "available",
          command: "codex-acp",
          binary_path: "/usr/local/bin/codex-acp",
          underlying_cli_path: "/usr/local/bin/codex",
          auth_status: { status: "logged_out" },
          login_hint: "Run `codex login` to authenticate.",
        },
        BUZZ_AGENT_AVAILABLE,
      ],
      connectAcpRuntimeDelayMs: 250,
      acpAuthMethods: {
        codex: {
          methods: [
            {
              id: "chat-gpt",
              name: "Sign in with ChatGPT",
              description: "Use your Codex subscription in the browser.",
              type: "browser",
            },
          ],
        },
      },
    });

    await page.goto("/", { waitUntil: "domcontentloaded" });
    await openSettings(page, "doctor");

    const row = page.getByTestId("doctor-runtime-codex");
    await expect(row).toBeVisible({ timeout: 10_000 });
    await expect(row).toContainText("Not authenticated");
    await expect(row).toContainText("Sign in with ChatGPT");
    await expect(row).toContainText(
      "Use your Codex subscription in the browser.",
    );
    await expect(row).toContainText("Credentials stay with Codex.");

    await row.getByRole("button", { name: "Sign in with ChatGPT" }).click();
    await expect(
      row.getByRole("button", { name: "Connecting..." }),
    ).toBeVisible({
      timeout: 5_000,
    });
  });

  /**
   * 07 — old or constrained adapter with no advertised auth methods: Doctor
   * falls back to manual instructions instead of inventing a login command.
   */
  test("07-connect-account-no-methods", async ({ page }) => {
    await installMockBridge(page, {
      acpRuntimesCatalog: [
        GOOSE_AVAILABLE,
        {
          ...CLAUDE_AVAILABLE_LOGGED_IN,
          auth_status: { status: "logged_out" },
          login_hint: "Run the Claude CLI to complete authentication.",
        },
        CODEX_NOT_INSTALLED,
        BUZZ_AGENT_AVAILABLE,
      ],
      acpAuthMethods: {
        claude: { methods: [] },
      },
    });

    await page.goto("/", { waitUntil: "domcontentloaded" });
    await openSettings(page, "doctor");

    const row = page.getByTestId("doctor-runtime-claude");
    await expect(row).toBeVisible({ timeout: 10_000 });
    await expect(row).toContainText("Not authenticated");
    await expect(row).toContainText(
      "This adapter did not advertise a built-in login flow.",
    );
    await expect(row).not.toContainText("Connect account");
  });
});
