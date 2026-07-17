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
   * 00 — the runtime catalog reads as a set of individual status cards rather
   * than one continuous table.
   */
  test("00-runtime-card-layout", async ({ page }) => {
    await installMockBridge(page, {
      acpRuntimesCatalog: [
        GOOSE_AVAILABLE,
        CLAUDE_AVAILABLE_LOGGED_IN,
        CODEX_NOT_INSTALLED,
        BUZZ_AGENT_AVAILABLE,
      ],
    });

    await page.goto("/", { waitUntil: "domcontentloaded" });
    await openSettings(page, "agents");

    const runtimeList = page.getByTestId("doctor-runtime-list");
    await expect(runtimeList).toBeVisible({ timeout: 10_000 });
    await expect(page.getByTestId("doctor-runtime-goose")).toBeVisible();
    await expect(page.getByTestId("doctor-runtime-codex")).toBeVisible();
    await expect(
      runtimeList.locator(":scope > [data-testid^='doctor-runtime-']"),
    ).toHaveCount(4);
    expect(
      await runtimeList
        .locator(":scope > [data-testid^='doctor-runtime-']")
        .evaluateAll((rows) =>
          rows.map((row) => row.getAttribute("data-testid")),
        ),
    ).toEqual([
      "doctor-runtime-buzz-agent",
      "doctor-runtime-goose",
      "doctor-runtime-claude",
      "doctor-runtime-codex",
    ]);
    for (const runtimeId of ["goose", "claude", "codex", "buzz-agent"]) {
      await expect(
        page.getByTestId(`doctor-runtime-logo-${runtimeId}`),
      ).toBeVisible();
    }
    const rowHeights = await Promise.all(
      ["goose", "claude", "codex", "buzz-agent"].map((runtimeId) =>
        page
          .getByTestId(`doctor-runtime-${runtimeId}`)
          .evaluate((element) =>
            Math.round(element.getBoundingClientRect().height),
          ),
      ),
    );
    expect(new Set(rowHeights).size).toBe(1);
    const [gooseColors, codexColors] = await Promise.all(
      ["goose", "codex"].map((runtimeId) =>
        page.getByTestId(`doctor-runtime-${runtimeId}`).evaluate((element) => {
          const styles = getComputedStyle(element);
          return {
            backgroundColor: styles.backgroundColor,
            borderColor: styles.borderColor,
          };
        }),
      ),
    );
    expect(codexColors).toEqual(gooseColors);
    await expect(
      page
        .getByRole("heading", { name: "Agent runtimes" })
        .locator("..")
        .locator(".."),
    ).toHaveCSS("align-items", "center");
    for (const runtimeId of ["goose", "claude", "buzz-agent"]) {
      await expect(
        page.getByTestId(`doctor-runtime-menu-${runtimeId}`),
      ).toHaveCount(0);
    }
    await expect(
      page.getByTestId("doctor-runtime-toggle-codex"),
    ).not.toBeChecked();
    await expect(page.getByTestId("doctor-runtime-toggle-codex")).toBeEnabled();
    for (const runtimeId of ["goose", "codex"]) {
      const toggle = page.getByTestId(`doctor-runtime-toggle-${runtimeId}`);
      await expect(toggle).toHaveClass(/shadow-none/);
      await expect(toggle.locator("span")).toHaveClass(/shadow-none/);
    }
    await expect(
      page.getByRole("menuitem", { name: "Instructions" }),
    ).toHaveCount(0);
    await page.getByTestId("doctor-runtime-menu-codex").click();
    await expect(
      page.getByRole("menuitem", { name: "Instructions" }),
    ).toBeVisible();
    await waitForAnimations(page);
    await page.screenshot({
      path: `${SHOTS}/00-runtime-overflow-menu.png`,
    });
    await page.keyboard.press("Escape");
    await expect(page.getByTestId("doctor-runtime-toggle-goose")).toBeChecked();
    await expect(
      page.getByTestId("doctor-runtime-toggle-goose"),
    ).toBeDisabled();
    await expect(page.getByTestId("doctor-runtime-codex")).not.toContainText(
      "Not installed",
    );

    await runtimeList.scrollIntoViewIfNeeded();
    await waitForAnimations(page);
    await runtimeList.screenshot({
      path: `${SHOTS}/00-runtime-card-layout.png`,
    });
  });

  /** 01 — a ready runtime stays compact without redundant status copy. */
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
    await openSettings(page, "agents");

    const row = page.getByTestId("doctor-runtime-claude");
    await expect(row).toBeVisible({ timeout: 10_000 });
    await expect(
      page.getByTestId("doctor-runtime-toggle-claude"),
    ).toBeChecked();
    await expect(row).not.toContainText("Authenticated");
    await expect(row).not.toContainText("Available");
    await expect(row).not.toContainText("claude-agent-acp");
    await expect(row).not.toContainText("/usr/local/bin");
    await expect(page.getByTestId("doctor-runtime-menu-claude")).toHaveCount(0);

    await row.scrollIntoViewIfNeeded();
    await waitForAnimations(page);
    await row.screenshot({ path: `${SHOTS}/01-auth-logged-in.png` });
  });

  /**
   * 02 — an available runtime that needs authentication stays the same height
   * as the others and moves setup instructions into its overflow menu.
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
    await openSettings(page, "agents");

    const row = page.getByTestId("doctor-runtime-codex");
    await expect(row).toBeVisible({ timeout: 10_000 });
    await expect(row).not.toContainText("Not authenticated");
    await expect(row).not.toContainText("Run `codex login` to authenticate.");
    await expect(row).toHaveCSS(
      "height",
      await page
        .getByTestId("doctor-runtime-goose")
        .evaluate((element) => getComputedStyle(element).height),
    );
    await page.getByTestId("doctor-runtime-menu-codex").click();
    await expect(
      page.getByRole("menuitem", { name: "Instructions" }),
    ).toBeVisible();
    await page.keyboard.press("Escape");

    await row.scrollIntoViewIfNeeded();
    await waitForAnimations(page);
    await row.screenshot({ path: `${SHOTS}/02-auth-logged-out.png` });
  });

  /**
   * 03 — a runtime with invalid configuration exposes its diagnostic and keeps
   * setup instructions in overflow.
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
    await openSettings(page, "agents");

    const row = page.getByTestId("doctor-runtime-claude");
    await expect(row).toBeVisible({ timeout: 10_000 });
    await expect(page.getByTestId("doctor-runtime-status-claude")).toHaveText(
      "Config error",
    );
    await expect(
      page.getByTestId("doctor-runtime-config-error-claude"),
    ).toContainText(
      "Config error: error loading configuration: ~/.claude/settings.json: unknown key foo",
    );
    await page.getByTestId("doctor-runtime-menu-claude").click();
    await expect(
      page.getByRole("menuitem", { name: "Instructions" }),
    ).toBeVisible();
    await page.keyboard.press("Escape");

    await row.scrollIntoViewIfNeeded();
    await waitForAnimations(page);
    await row.screenshot({ path: `${SHOTS}/03-auth-config-error.png` });
  });

  /**
   * 04 — adapter_missing runtime with node_required: true: the off toggle is
   * disabled, and the Node.js action moves into the overflow menu.
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
    await openSettings(page, "agents");

    const row = page.getByTestId("doctor-runtime-codex");
    await expect(row).toBeVisible({ timeout: 10_000 });
    const toggle = page.getByTestId("doctor-runtime-toggle-codex");
    await expect(toggle).not.toBeChecked();
    await expect(toggle).toBeDisabled();
    await expect(page.getByTestId("doctor-runtime-status-codex")).toHaveText(
      "Adapter needed",
    );
    await expect(row).not.toContainText("Node.js is required");
    await expect(row).toHaveCSS(
      "height",
      await page
        .getByTestId("doctor-runtime-goose")
        .evaluate((element) => getComputedStyle(element).height),
    );
    await page.getByTestId("doctor-runtime-menu-codex").click();
    await expect(
      page.getByRole("menuitem", { name: "Install Node.js" }),
    ).toBeVisible();
    await page.keyboard.press("Escape");

    await row.scrollIntoViewIfNeeded();
    await waitForAnimations(page);
    await row.screenshot({ path: `${SHOTS}/04-node-required.png` });
  });

  /**
   * 05 — a failed toggle install returns to off; toggling again retries.
   *
   * The mock is configured with a two-call sequence:
   *   call 1 → failure (E404)
   *   call 2 → success
   * This exercises the full retry path: fail state → toggle on again →
   * success banner.
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
      installAcpRuntimeDelayMs: 250,
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
    await openSettings(page, "agents");

    const row = page.getByTestId("doctor-runtime-codex");
    await expect(row).toBeVisible({ timeout: 10_000 });
    await expect(row).not.toContainText("Not installed");

    // Trigger the first install — the mock returns a failure.
    const toggle = page.getByTestId("doctor-runtime-toggle-codex");
    await expect(toggle).not.toBeChecked();
    await expect(toggle).toBeEnabled();
    await toggle.click();
    const loading = page.getByTestId("doctor-runtime-loading-codex");
    await expect(loading).toBeVisible();
    await expect(loading).toContainText("Codex installing");
    await expect(toggle).toHaveCount(0);

    // After failure: the toggle returns to off and the error is visible.
    await expect(loading).toHaveCount(0, { timeout: 5_000 });
    await expect(toggle).not.toBeChecked({ timeout: 5_000 });
    await expect(toggle).toBeEnabled();
    await expect(row).toContainText("Step");
    await expect(row).toContainText("failed");

    await row.scrollIntoViewIfNeeded();
    await waitForAnimations(page);
    await row.screenshot({ path: `${SHOTS}/05-retry-after-failure.png` });

    // Toggle on again — the mock returns success on the second call.
    await toggle.click();
    await expect(loading).toBeVisible();
    await expect(toggle).toHaveCount(0);

    // The error disappears, then the success banner and on state render.
    await expect(loading).toHaveCount(0, { timeout: 5_000 });
    await expect(row).not.toContainText("failed", { timeout: 5_000 });
    await expect(
      row.getByText("Codex installed. Checking for sign-in options..."),
    ).toBeVisible({
      timeout: 10_000,
    });
    await expect(toggle).toBeChecked();
    await expect(toggle).toBeDisabled();

    await row.scrollIntoViewIfNeeded();
    await waitForAnimations(page);
    await row.screenshot({ path: `${SHOTS}/05-retry-success.png` });
  });

  /**
   * 06 — adapter-provided account methods appear in the overflow menu and
   * launch the vendor-owned flow without expanding the runtime row.
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
    await openSettings(page, "agents");

    const row = page.getByTestId("doctor-runtime-codex");
    await expect(row).toBeVisible({ timeout: 10_000 });
    await expect(row).not.toContainText("Not authenticated");
    await expect(row).toHaveCSS(
      "height",
      await page
        .getByTestId("doctor-runtime-goose")
        .evaluate((element) => getComputedStyle(element).height),
    );
    await page.getByTestId("doctor-runtime-menu-codex").click();
    await expect(
      page.getByRole("menuitem", { name: "Sign in with ChatGPT" }),
    ).toBeVisible({
      timeout: 5_000,
    });
    await page.getByRole("menuitem", { name: "Sign in with ChatGPT" }).click();
    const loading = page.getByTestId("doctor-runtime-loading-codex");
    await expect(loading).toBeVisible();
    await expect(loading).toContainText("Codex connecting");
    await expect(page.getByTestId("doctor-runtime-toggle-codex")).toHaveCount(
      0,
    );
    await expect(loading).toHaveCount(0, { timeout: 5_000 });
    await expect(page.getByTestId("doctor-runtime-toggle-codex")).toBeChecked();
  });

  /**
   * 07 — an adapter with no advertised auth methods shows only its manual
   * instructions in overflow and keeps the row compact.
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
    await openSettings(page, "agents");

    const row = page.getByTestId("doctor-runtime-claude");
    await expect(row).toBeVisible({ timeout: 10_000 });
    await expect(row).not.toContainText("Not authenticated");
    await expect(row).toHaveCSS(
      "height",
      await page
        .getByTestId("doctor-runtime-goose")
        .evaluate((element) => getComputedStyle(element).height),
    );
    await page.getByTestId("doctor-runtime-menu-claude").click();
    await expect(
      page.getByRole("menuitem", { name: "Instructions" }),
    ).toBeVisible();
    await expect(
      page.getByRole("menuitem", { name: "Sign in with ChatGPT" }),
    ).toHaveCount(0);
  });

  test("08-auth-method-discovery-error", async ({ page }) => {
    await installMockBridge(page, {
      acpRuntimesCatalog: [
        GOOSE_AVAILABLE,
        CLAUDE_AVAILABLE_LOGGED_IN,
        {
          ...CODEX_NOT_INSTALLED,
          availability: "available",
          auth_status: { status: "logged_out" },
        },
        BUZZ_AGENT_AVAILABLE,
      ],
      acpAuthMethodsErrors: {
        codex: "Could not inspect the Codex adapter.",
      },
    });

    await page.goto("/", { waitUntil: "domcontentloaded" });
    await openSettings(page, "agents");

    await expect(page.getByTestId("doctor-runtime-error-codex")).toContainText(
      "Couldn't load sign-in options: Could not inspect the Codex adapter.",
    );
  });

  test("09-connect-account-error", async ({ page }) => {
    await installMockBridge(page, {
      acpRuntimesCatalog: [
        GOOSE_AVAILABLE,
        CLAUDE_AVAILABLE_LOGGED_IN,
        {
          ...CODEX_NOT_INSTALLED,
          availability: "available",
          auth_status: { status: "logged_out" },
        },
        BUZZ_AGENT_AVAILABLE,
      ],
      acpAuthMethods: {
        codex: {
          methods: [
            {
              id: "chat-gpt",
              name: "Sign in with ChatGPT",
              type: "browser",
            },
          ],
        },
      },
      connectAcpRuntimeError: "The browser could not be opened.",
    });

    await page.goto("/", { waitUntil: "domcontentloaded" });
    await openSettings(page, "agents");

    await page.getByTestId("doctor-runtime-menu-codex").click();
    await page.getByRole("menuitem", { name: "Sign in with ChatGPT" }).click();
    await expect(page.getByTestId("doctor-runtime-error-codex")).toContainText(
      "Couldn't connect Codex: The browser could not be opened.",
    );
  });

  test("10-terminal-auth-completion-guidance", async ({ page }) => {
    await installMockBridge(page, {
      acpRuntimesCatalog: [
        GOOSE_AVAILABLE,
        CLAUDE_AVAILABLE_LOGGED_IN,
        {
          ...CODEX_NOT_INSTALLED,
          availability: "available",
          auth_status: { status: "logged_out" },
        },
        BUZZ_AGENT_AVAILABLE,
      ],
      acpAuthMethods: {
        codex: {
          methods: [
            {
              id: "terminal-login",
              name: "Sign in from Terminal",
              type: "terminal",
            },
          ],
        },
      },
      connectAcpRuntimeResult: { launched: true },
    });

    await page.goto("/", { waitUntil: "domcontentloaded" });
    await openSettings(page, "agents");

    await page.getByTestId("doctor-runtime-menu-codex").click();
    await page.getByRole("menuitem", { name: "Sign in from Terminal" }).click();
    await expect(
      page.getByTestId("doctor-runtime-terminal-guidance-codex"),
    ).toContainText(
      "Finish signing in from the Terminal window, then click Check again to re-check Codex.",
    );
  });

  test("11-outdated-adapter-warning", async ({ page }) => {
    await installMockBridge(page, {
      acpRuntimesCatalog: [
        GOOSE_AVAILABLE,
        CLAUDE_AVAILABLE_LOGGED_IN,
        {
          ...CODEX_NOT_INSTALLED,
          availability: "adapter_outdated",
          binary_path: "/usr/local/bin/codex-acp",
          underlying_cli_path: "/usr/local/bin/codex",
          can_auto_install: true,
        },
        BUZZ_AGENT_AVAILABLE,
      ],
      installAcpRuntimeDelayMs: 250,
    });

    await page.goto("/", { waitUntil: "domcontentloaded" });
    await openSettings(page, "agents");

    await expect(page.getByTestId("doctor-runtime-status-codex")).toHaveText(
      "Update needed",
    );
    await page.getByTestId("doctor-runtime-toggle-codex").click();

    const dialog = page.getByRole("alertdialog");
    await expect(dialog).toContainText("Update Codex adapter?");
    await expect(dialog).toContainText(
      "Older Buzz releases using the legacy adapter may lose community access",
    );
    await expect(page.getByTestId("doctor-runtime-loading-codex")).toHaveCount(
      0,
    );

    await page.getByTestId("doctor-runtime-confirm-update-codex").click();
    const loading = page.getByTestId("doctor-runtime-loading-codex");
    await expect(loading).toBeVisible();
    await expect(loading).toContainText("Codex installing");
  });
});
