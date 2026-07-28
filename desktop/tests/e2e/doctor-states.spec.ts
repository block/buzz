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
  install_instructions_url:
    "https://goose-docs.ai/docs/getting-started/installation/",
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
  install_hint: "Buzz talks to Codex through the Codex CLI.",
  install_instructions_url: "https://developers.openai.com/codex/cli/",
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
    expect(rowHeights[2]).toBeGreaterThan(rowHeights[0]);
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
        .getByRole("heading", { name: "Agent runtimes", exact: true })
        .locator("..")
        .locator(".."),
    ).toHaveCSS("align-items", "center");
    for (const runtimeId of ["goose", "claude", "buzz-agent"]) {
      await expect(
        page.getByTestId(`doctor-runtime-menu-${runtimeId}`),
      ).toHaveCount(0);
    }
    const codexInstallButton = page.getByTestId("doctor-runtime-install-codex");
    await expect(codexInstallButton).toBeEnabled();
    await expect(codexInstallButton).toHaveText("Install");
    await expect(
      page.getByRole("menuitem", { name: "CLI setup guide" }),
    ).toHaveCount(0);
    await page.getByTestId("doctor-runtime-menu-codex").click();
    await expect(
      page.getByRole("menuitem", { name: "CLI setup guide" }),
    ).toBeVisible();
    await waitForAnimations(page);
    await page.screenshot({
      path: `${SHOTS}/00-runtime-overflow-menu.png`,
    });
    await page.keyboard.press("Escape");
    await expect(page.getByTestId("doctor-runtime-ready-goose")).toHaveText(
      "Ready",
    );
    await expect(page.getByTestId("doctor-runtime-install-goose")).toHaveCount(
      0,
    );
    await expect(page.getByTestId("doctor-runtime-codex")).not.toContainText(
      "Not installed",
    );
    await expect(page.getByTestId("doctor-runtime-status-codex")).toHaveText(
      "CLI needed",
    );
    await expect(
      page.getByTestId("doctor-runtime-guidance-codex"),
    ).toContainText("Buzz talks to Codex through the Codex CLI.");
    await expect(
      page
        .getByTestId("doctor-runtime-guidance-codex")
        .getByRole("button", { name: "CLI setup guide" }),
    ).toBeVisible();

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
    await expect(page.getByTestId("doctor-runtime-ready-claude")).toHaveText(
      "Ready",
    );
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
   * 02 — an available runtime that needs authentication shows an explicit
   * "Sign-in needed" chip on the row face (never a green Ready chip), stays
   * the same height as the others, and keeps setup instructions in its
   * overflow menu.
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
    // Auth-required is an explicit row-face state: amber "Sign-in needed"
    // chip (single source: entryStatusLabel), no green Ready chip.
    await expect(page.getByTestId("doctor-runtime-status-codex")).toHaveText(
      "Sign-in needed",
    );
    await expect(page.getByTestId("doctor-runtime-ready-codex")).toHaveCount(0);
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
      page.getByRole("menuitem", { name: "CLI setup guide" }),
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
      page.getByRole("menuitem", { name: "CLI setup guide" }),
    ).toBeVisible();
    await page.keyboard.press("Escape");

    await row.scrollIntoViewIfNeeded();
    await waitForAnimations(page);
    await row.screenshot({ path: `${SHOTS}/03-auth-config-error.png` });
  });

  /**
   * 04 — adapter_missing runtime with node_required: true is NOT a
   * one-click-ready row: it must be catalog-only (no Install button), and its
   * catalog detail must offer the setup guide instead of a one-click Install
   * that would fail without Node.
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
          install_hint: "Install the Codex ACP adapter via npm.",
          install_instructions_url:
            "https://github.com/agentclientprotocol/codex-acp",
        },
        BUZZ_AGENT_AVAILABLE,
      ],
    });

    await page.goto("/", { waitUntil: "domcontentloaded" });
    await openSettings(page, "agents");

    // Node-gated entries never get a Your-harnesses row (and thus never an
    // Install button) — setup happens in the catalog.
    await expect(page.getByTestId("doctor-runtime-goose")).toBeVisible({
      timeout: 10_000,
    });
    await expect(page.getByTestId("doctor-runtime-codex")).toHaveCount(0);
    await expect(page.getByTestId("doctor-runtime-install-codex")).toHaveCount(
      0,
    );
    await expect(page.getByTestId("doctor-runtime-ready-codex")).toHaveCount(0);

    await page.getByTestId("harness-add-button").click();
    await expect(page.getByTestId("harness-catalog-dialog")).toBeVisible();
    await page.getByTestId("harness-catalog-list-item-codex").click();
    await expect(page.getByTestId("harness-catalog-status-codex")).toHaveText(
      "Adapter needed",
    );
    // Node gate blocks one-click install; the primary action is the vendor
    // setup guide instead — a single pinned bottom-bar CTA
    // (harness-catalog-setup-*), not a separate docs button.
    await expect(page.getByTestId("harness-catalog-install-codex")).toHaveCount(
      0,
    );
    await expect(page.getByTestId("harness-catalog-setup-codex")).toBeVisible();
    await expect(page.getByTestId("harness-catalog-setup-codex")).toHaveText(
      /Setup guide/,
    );
    await expect(page.getByTestId("harness-catalog-docs-codex")).toHaveCount(0);

    const detail = page.getByTestId("harness-catalog-detail-pane");
    await expect(detail).toContainText("Install the Codex ACP adapter");
    await waitForAnimations(page);
    await detail.screenshot({ path: `${SHOTS}/04-node-required.png` });
  });

  /**
   * 05 — a failed install brings the Install button back; clicking it again
   * retries.
   *
   * The mock is configured with a two-call sequence:
   *   call 1 → failure (E404)
   *   call 2 → installer exit 0, but post-install discovery still cannot find
   *            the runtime
   * This exercises the full retry path: fail state → Install again →
   * verified failure without a false installed state.
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
          success: false,
          steps: [
            {
              step: "cli",
              command: "powershell.exe install codex",
              success: true,
              stdout: "installed",
              stderr: "",
              exit_code: 0,
            },
            {
              step: "verify",
              command: "discover codex",
              success: false,
              stdout: "",
              stderr:
                "The installer finished, but Buzz still could not use codex (observed: NotInstalled).",
              exit_code: null,
              hint: "Buzz requires the vendor CLI executable, not only its desktop app. If the CLI was installed while Buzz was open, restart Buzz and check again.",
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
    const installButton = page.getByTestId("doctor-runtime-install-codex");
    await expect(installButton).toBeEnabled();
    await expect(installButton).toHaveText("Install");
    await installButton.click();
    const loading = page.getByTestId("doctor-runtime-loading-codex");
    await expect(loading).toBeVisible();
    await expect(loading).toContainText("Codex installing");
    await expect(installButton).toHaveCount(0);

    // After failure: the Install button returns and the error is visible.
    await expect(loading).toHaveCount(0, { timeout: 5_000 });
    await expect(installButton).toBeVisible({ timeout: 5_000 });
    await expect(installButton).toBeEnabled();
    await expect(row).toContainText("Step");
    await expect(row).toContainText("failed");

    await row.scrollIntoViewIfNeeded();
    await waitForAnimations(page);
    await row.screenshot({ path: `${SHOTS}/05-retry-after-failure.png` });

    // Install again — the install command exits 0, but verification fails.
    await installButton.click();
    await expect(loading).toBeVisible();
    await expect(installButton).toHaveCount(0);

    // The runtime remains retryable and never renders a false success state.
    await expect(loading).toHaveCount(0, { timeout: 5_000 });
    await expect(row).toContainText("desktop app", { timeout: 5_000 });
    await expect(row).toContainText('Step "verify" failed');
    await expect(row.getByText(/installed\. Checking/)).toHaveCount(0);
    await expect(installButton).toBeVisible();
    await expect(installButton).toBeEnabled();

    await row.scrollIntoViewIfNeeded();
    await waitForAnimations(page);
    await row.screenshot({ path: `${SHOTS}/05-verification-failure.png` });
  });

  test("05b-verified-install-enables-runtime", async ({ page }) => {
    await installMockBridge(page, {
      acpRuntimesCatalog: [
        GOOSE_AVAILABLE,
        CLAUDE_AVAILABLE_LOGGED_IN,
        CODEX_NOT_INSTALLED,
        BUZZ_AGENT_AVAILABLE,
      ],
      acpRuntimesCatalogAfterInstall: [
        GOOSE_AVAILABLE,
        CLAUDE_AVAILABLE_LOGGED_IN,
        {
          ...CODEX_NOT_INSTALLED,
          availability: "available",
          command: "codex-acp",
          binary_path: "/usr/local/bin/codex-acp",
          underlying_cli_path: "/usr/local/bin/codex",
          auth_status: { status: "logged_in" },
        },
        BUZZ_AGENT_AVAILABLE,
      ],
      installAcpRuntimeResult: {
        success: true,
        steps: [
          {
            step: "adapter",
            command: "npm install -g @agentclientprotocol/codex-acp",
            success: true,
            stdout: "added 1 package",
            stderr: "",
            exit_code: 0,
          },
        ],
      },
    });

    await page.goto("/", { waitUntil: "domcontentloaded" });
    await openSettings(page, "agents");

    const installButton = page.getByTestId("doctor-runtime-install-codex");
    await expect(installButton).toBeEnabled();
    await installButton.click();
    await expect(page.getByTestId("doctor-runtime-ready-codex")).toBeVisible({
      timeout: 5_000,
    });
    await expect(installButton).toHaveCount(0);
    await expect(page.getByTestId("doctor-runtime-guidance-codex")).toHaveCount(
      0,
    );
  });

  /**
   * 06 — adapter-provided account methods appear in the overflow menu and
   * launch the vendor-owned flow without expanding the runtime row. The
   * row face flips "Sign-in needed" → Ready once the connect settles and
   * discovery reports logged_in.
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
      // After the mocked connect succeeds, discovery reports logged_in so
      // the row face can flip from "Sign-in needed" to Ready.
      acpRuntimesCatalogAfterConnect: [
        GOOSE_AVAILABLE,
        CLAUDE_AVAILABLE_LOGGED_IN,
        {
          ...CODEX_NOT_INSTALLED,
          availability: "available",
          command: "codex-acp",
          binary_path: "/usr/local/bin/codex-acp",
          underlying_cli_path: "/usr/local/bin/codex",
          auth_status: { status: "logged_in" },
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
    // Signed-out row face: explicit auth chip, no green Ready chip.
    await expect(page.getByTestId("doctor-runtime-status-codex")).toHaveText(
      "Sign-in needed",
    );
    await expect(page.getByTestId("doctor-runtime-ready-codex")).toHaveCount(0);
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
    await expect(page.getByTestId("doctor-runtime-ready-codex")).toHaveCount(0);
    await expect(loading).toHaveCount(0, { timeout: 5_000 });
    // Connect settled and discovery reports logged_in — sign-in chip gone,
    // Ready chip on.
    await expect(page.getByTestId("doctor-runtime-ready-codex")).toBeVisible();
    await expect(page.getByTestId("doctor-runtime-status-codex")).toHaveCount(
      0,
    );
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
      page.getByRole("menuitem", { name: "CLI setup guide" }),
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
    const updateButton = page.getByTestId("doctor-runtime-install-codex");
    await expect(updateButton).toHaveText("Update");
    await updateButton.click();

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

  /**
   * 08 — concurrent installs each keep their own spinner/result state;
   *      stale install failure is cleared when Check again fires (F1 fix).
   *
   * Flow:
   *  - Claude (400ms delay) → failure
   *  - Codex  (100ms delay) → success
   *  Both started before either settles.
   *  After both settle: claude shows failure, codex shows success banner.
   *  Click Check again → both rows lose stale state (claude error gone).
   */
  test("08-concurrent-installs-and-stale-clear", async ({ page }) => {
    await installMockBridge(page, {
      acpRuntimesCatalog: [
        {
          ...CLAUDE_AVAILABLE_LOGGED_IN,
          availability: "adapter_missing",
          command: null,
          binary_path: null,
          can_auto_install: true,
          auth_status: { status: "unknown" },
        },
        {
          ...CODEX_NOT_INSTALLED,
          can_auto_install: true,
          node_required: false,
        },
        GOOSE_AVAILABLE,
        BUZZ_AGENT_AVAILABLE,
      ],
      installAcpRuntimeByRuntime: {
        claude: {
          delayMs: 400,
          result: {
            success: false,
            steps: [
              {
                step: "adapter",
                command: "npm install -g @agentclientprotocol/claude-agent-acp",
                success: false,
                stdout: "",
                stderr:
                  "npm ERR! code EACCES\nnpm ERR! syscall mkdir\nnpm ERR! path /usr/local\n\nHint: Check prefix permissions.",
                exit_code: 1,
              },
            ],
          },
        },
        codex: {
          delayMs: 100,
          result: {
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
        },
      },
      // After the catalog refresh (triggered by a successful install or Check
      // again), all runtimes report healthy so stale errors must clear.
      acpRuntimesCatalogAfterInstall: [
        {
          ...CLAUDE_AVAILABLE_LOGGED_IN,
          availability: "available",
        },
        {
          ...CODEX_NOT_INSTALLED,
          availability: "available",
          command: "codex-acp",
          binary_path: "/usr/local/bin/codex-acp",
          auth_status: { status: "logged_in" },
        },
        GOOSE_AVAILABLE,
        BUZZ_AGENT_AVAILABLE,
      ],
    });

    await page.goto("/", { waitUntil: "domcontentloaded" });
    await openSettings(page, "agents");

    const claudeRow = page.getByTestId("doctor-runtime-claude");
    const codexRow = page.getByTestId("doctor-runtime-codex");
    await expect(claudeRow).toBeVisible({ timeout: 10_000 });
    await expect(codexRow).toBeVisible();

    const claudeInstallButton = page.getByTestId(
      "doctor-runtime-install-claude",
    );
    const codexInstallButton = page.getByTestId("doctor-runtime-install-codex");
    const codexReadyChip = page.getByTestId("doctor-runtime-ready-codex");

    // Start both installs before either settles.
    await claudeInstallButton.click();
    await codexInstallButton.click();

    // Codex settles first (shorter delay): Ready chip appears, no error on
    // codex. The catalog refresh triggered by codex's success immediately
    // returns availability === "available", so the transient "installed.
    // Checking..." banner is replaced by the stable ready state — assert the
    // Ready chip instead.
    await expect(codexReadyChip).toBeVisible({ timeout: 3_000 });
    await expect(
      page.getByTestId("doctor-runtime-install-error-codex"),
    ).toHaveCount(0);

    // Claude settles (after its longer delay): failure error visible with
    // multiline stderr. Codex must still be ready — unaffected by claude.
    const claudeError = page.getByTestId("doctor-runtime-install-error-claude");
    await expect(claudeError).toBeVisible({ timeout: 3_000 });
    await expect(claudeError).toContainText("npm ERR!");
    await expect(codexReadyChip).toBeVisible();

    // Click Check again — epoch increments, RuntimeRow useEffect clears
    // local installResult state, so the stale claude error disappears.
    await page.getByRole("button", { name: "Check again" }).click();
    await expect(claudeError).toHaveCount(0, { timeout: 5_000 });
    // Codex stays ready (catalog still reports available after refresh).
    await expect(codexReadyChip).toBeVisible({ timeout: 5_000 });

    await claudeRow.scrollIntoViewIfNeeded();
    await waitForAnimations(page);
    await claudeRow.screenshot({
      path: `${SHOTS}/08-concurrent-installs-and-stale-clear.png`,
    });
  });
});
