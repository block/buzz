import { expect, test, type Page } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const CHANNEL_ID = "94a444a4-c0a3-5966-ab05-530c6ddc2301";
const REQUIRED_SENSITIVE_EXCLUDES = [
  ".env*",
  "**/.env*",
  ".npmrc",
  "**/.npmrc",
  ".netrc",
  "**/.netrc",
  ".pypirc",
  "**/.pypirc",
  ".git-credentials",
  "**/.git-credentials",
  ".git/**",
  "nxtlinq/**",
  ".aws/**",
  "**/.aws/**",
  ".docker/**",
  "**/.docker/**",
  "credentials",
  "**/credentials",
  "**/credentials/**",
  "**/.ssh/**",
  "*.pem",
  "**/*.pem",
  "*.key",
  "**/*.key",
  "*.p12",
  "**/*.p12",
];

async function openNxtlinqSetupDraft(page: Page, agentPubkey: string) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_SEED_LIVE_OBSERVER_EVENTS__ === "function",
  );
  await page.getByTestId("channel-agents").click();
  await expect(page.getByTestId("chat-title")).toHaveText("agents");

  await page.evaluate(
    ({ agentPubkey, channelId, sensitiveExcludes }) => {
      window.__BUZZ_E2E_SEED_LIVE_OBSERVER_EVENTS__?.({
        agentPubkey,
        events: [
          {
            seq: 1,
            timestamp: new Date().toISOString(),
            kind: "agent_management_request",
            agentIndex: 0,
            channelId,
            sessionId: "session-nxtlinq",
            turnId: "turn-nxtlinq",
            payload: {
              type: "agent_management_request",
              action: "nxtlinq_setup",
              requestId: "request-nxtlinq",
              request: {
                channelId,
                projectRoot: "/tmp/mock-project",
                explanation:
                  "Read documentation and source while excluding .env and signing material.",
                policy: {
                  name: "policy-helper",
                  version: "1.0.0",
                  scope: ["demo:structured-capabilities"],
                  aud: ["nxtlinq-authorization-gateway"],
                  capabilities: [
                    {
                      type: "filesystem:read",
                      include: ["README.md", "src/**"],
                      exclude: sensitiveExcludes,
                    },
                    {
                      type: "mcp:connect",
                      servers: ["buzz-dev-mcp"],
                    },
                  ],
                },
              },
            },
          },
        ],
      });
    },
    {
      agentPubkey,
      channelId: CHANNEL_ID,
      sensitiveExcludes: REQUIRED_SENSITIVE_EXCLUDES,
    },
  );
}

test("an owned Agent opens a policy-only Nxtlinq review draft", async ({
  page,
}) => {
  const agent = TEST_IDENTITIES.charlie;
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: agent.pubkey,
        name: "Policy helper",
        status: "running",
        channelIds: [CHANNEL_ID],
      },
    ],
    nxtlinqAuthorizationConfig: {
      trustStore: "/tmp/operator/trusted-signers.json",
      receiptRoot: "/tmp/operator/receipts",
    },
  });
  await openNxtlinqSetupDraft(page, agent.pubkey);

  const dialog = page.getByTestId("nxtlinq-setup-review-dialog");
  await expect(dialog).toBeVisible();
  const progress = dialog.getByTestId("nxtlinq-setup-progress");
  for (const label of ["Project", "Policy", "Activate"]) {
    await expect(progress.getByText(label, { exact: true })).toBeVisible();
  }
  await expect(progress.getByText("Local trust", { exact: true })).toHaveCount(
    0,
  );
  await expect(dialog.getByTestId("nxtlinq-workspace-step")).toBeVisible();
  await expect(dialog.getByTestId("nxtlinq-trust-step")).toHaveCount(0);
  await expect(
    dialog.getByTestId("nxtlinq-manifest-policy-editor"),
  ).toHaveCount(0);
  await expect(dialog).toContainText(
    "Initialization, signing, and enablement are locked until you explicitly use this project",
  );
  await dialog.getByRole("button", { name: "Use as Agent workspace" }).click();
  await expect(dialog).toContainText("This is the Agent workspace");
  await expect(dialog).toContainText("Nxtlinq Attest is initialized");
  await dialog.getByRole("button", { name: "Continue to policy" }).click();

  await expect(dialog).toContainText("excluding .env and signing material");
  await expect(dialog).toContainText("filesystem:read");
  await expect(dialog).toContainText("mcp:connect");
  await expect(dialog).not.toContainText("terminal:execute");
  await expect(dialog).not.toContainText("mcp:invoke");
  await expect(
    dialog.getByLabel("Editable Nxtlinq permission proposal"),
  ).toBeEditable();
  await expect(dialog).toContainText("Locked safeguards");
  await expect(dialog).toContainText(
    "These protections are added by Buzz and cannot be removed",
  );
  await expect(dialog).toContainText("Current manifest is shown on the left");
  await expect(
    dialog.getByText("Current manifest", { exact: true }),
  ).toBeVisible();
  await expect(
    dialog.getByText("Proposed manifest", { exact: true }),
  ).toBeVisible();
  await expect(
    dialog.getByTestId("nxtlinq-manifest-diff").locator(".buzz-diff-table"),
  ).toHaveClass(/min-w-\[780px\]/);
  const changedFragment = dialog
    .getByTestId("nxtlinq-manifest-diff")
    .locator(".diff-code-edit")
    .first();
  await expect(changedFragment).toBeVisible();
  const diffColors = await changedFragment.evaluate((fragment) => {
    const line = fragment.closest(".buzz-diff-code");
    return {
      fragment: getComputedStyle(fragment).backgroundColor,
      line: line ? getComputedStyle(line).backgroundColor : null,
    };
  });
  expect(diffColors.line).toBe("rgba(0, 0, 0, 0)");
  expect(diffColors.fragment).not.toBe(diffColors.line);

  const editor = dialog.getByTestId("nxtlinq-manifest-policy-editor");
  const editedPolicy = JSON.parse(await editor.inputValue());
  expect(editedPolicy).not.toHaveProperty("scope");
  expect(editedPolicy).not.toHaveProperty("aud");
  const invalidPolicy = {
    ...editedPolicy,
    scope: ["owner:invented-scope"],
  };
  await editor.fill(JSON.stringify(invalidPolicy, null, 2));
  await expect(dialog.getByRole("alert")).toContainText(
    "The editable policy is not valid",
  );

  editedPolicy.name = "owner-reviewed-policy";
  await editor.fill(JSON.stringify(editedPolicy, null, 2));
  await expect(dialog).toContainText("Checking your edits and updating");
  await expect(dialog).toContainText("owner-reviewed-policy");

  const applyManifestButton = dialog.getByRole("button", {
    name: "Apply manifest changes",
  });
  await expect(applyManifestButton).toBeDisabled();
  await dialog
    .getByLabel("I reviewed the current and proposed manifest shown above.")
    .check();
  await expect(applyManifestButton).toBeEnabled();
  await applyManifestButton.click();
  await expect(dialog).toContainText("owner key in secure storage");
  await expect(dialog).not.toContainText("privateKey");

  await dialog.getByRole("button", { name: "Sign manifest securely" }).click();
  await expect(dialog).toContainText("signed and trusted as mock-signer");
  await expect(dialog).not.toContainText("owner-private.key");

  await dialog.getByRole("button", { name: "Recheck & enable Agent" }).click();
  await expect(dialog).toContainText("The Agent remains stopped");
});

test("an uninitialized project can create a managed signing identity in the review", async ({
  page,
}) => {
  const agent = TEST_IDENTITIES.charlie;
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: agent.pubkey,
        name: "Policy helper",
        status: "running",
        channelIds: [CHANNEL_ID],
        workingDirectory: "/tmp/mock-project",
      },
    ],
    nxtlinqAuthorizationConfig: {
      trustStore: "/tmp/operator/trusted-signers.json",
      receiptRoot: "/tmp/operator/receipts",
    },
    nxtlinqAttestInitialization: {
      status: "missing",
      detail: null,
    },
  });
  await openNxtlinqSetupDraft(page, agent.pubkey);

  const dialog = page.getByTestId("nxtlinq-setup-review-dialog");
  await expect(dialog.getByTestId("nxtlinq-workspace-step")).toBeVisible();
  const initialization = dialog.getByTestId("nxtlinq-project-initialization");
  await expect(initialization).toBeVisible();
  await expect(initialization).toContainText("Initialize Nxtlinq Attest");
  await expect(initialization).toContainText(
    "protect the private key in secure storage",
  );
  await expect(initialization).toContainText(
    "stores the owner key in the system keychain",
  );
  await expect(initialization).toContainText(
    "my-project-owner. Do not enter a key or file path",
  );
  await expect(
    initialization.getByPlaceholder("my-project-policy-2026"),
  ).toBeVisible();
  await expect(dialog.getByTestId("nxtlinq-manifest-diff")).toHaveCount(0);
  await expect(
    dialog.getByRole("button", {
      name: "Initialize securely",
    }),
  ).toBeEnabled();

  await dialog
    .getByRole("button", {
      name: "Initialize securely",
    })
    .click();

  await expect(initialization).toHaveCount(0);
  await expect(dialog).toContainText(
    "initialized with a new owner-controlled signing key",
  );
  await expect(dialog).toContainText("System secure storage");
  await expect(dialog).toContainText("sha256:mock-public-key-fingerprint");
  await expect(dialog).toContainText("Public signer enrolled");
  await expect(dialog.getByTestId("nxtlinq-manifest-diff")).toBeVisible();

  const rendered = (await dialog.textContent()) ?? "";
  expect(rendered).not.toContain("owner-private.key");
  expect(rendered).not.toContain("/tmp/operator/private.key");
  expect(rendered).not.toContain("BEGIN PRIVATE KEY");
});

test("an Agent draft can request and receive a regenerated proposal", async ({
  page,
}) => {
  const agent = TEST_IDENTITIES.charlie;
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: agent.pubkey,
        name: "Policy helper",
        status: "running",
        channelIds: [CHANNEL_ID],
      },
    ],
    nxtlinqAuthorizationConfig: {
      trustStore: "/tmp/operator/trusted-signers.json",
      receiptRoot: "/tmp/operator/receipts",
    },
  });
  await openNxtlinqSetupDraft(page, agent.pubkey);

  let dialog = page.getByTestId("nxtlinq-setup-review-dialog");
  await dialog.getByRole("button", { name: "Use as Agent workspace" }).click();
  await dialog.getByRole("button", { name: "Continue to policy" }).click();
  await dialog
    .getByLabel("Optional guidance for regenerated Nxtlinq proposal")
    .fill("Limit ordinary reads to src/** only.");
  await dialog.getByRole("button", { name: "Regenerate proposal" }).click();
  await expect(dialog).toContainText(
    "Request sent. Waiting for the Agent's new Desktop draft",
  );
  await expect(
    dialog.getByLabel("Editable Nxtlinq permission proposal"),
  ).toBeDisabled();
  await expect(
    dialog.getByRole("button", { name: "Apply manifest changes" }),
  ).toBeDisabled();

  await page.evaluate(
    ({ agentPubkey, channelId, sensitiveExcludes }) => {
      window.__BUZZ_E2E_SEED_LIVE_OBSERVER_EVENTS__?.({
        agentPubkey,
        events: [
          {
            seq: 2,
            timestamp: new Date().toISOString(),
            kind: "agent_management_request",
            agentIndex: 0,
            channelId,
            sessionId: "session-nxtlinq-regenerated",
            turnId: "turn-nxtlinq-regenerated",
            payload: {
              type: "agent_management_request",
              action: "nxtlinq_setup",
              requestId: "request-nxtlinq-regenerated",
              request: {
                channelId,
                projectRoot: "/tmp/mock-project",
                explanation: "Regenerated narrow source-only proposal.",
                policy: {
                  name: "regenerated-policy",
                  version: "1.0.0",
                  scope: ["demo:structured-capabilities"],
                  aud: ["nxtlinq-authorization-gateway"],
                  capabilities: [
                    {
                      type: "filesystem:read",
                      include: ["src/**"],
                      exclude: sensitiveExcludes,
                    },
                    {
                      type: "mcp:connect",
                      servers: ["buzz-dev-mcp"],
                    },
                  ],
                },
              },
            },
          },
        ],
      });
    },
    {
      agentPubkey: agent.pubkey,
      channelId: CHANNEL_ID,
      sensitiveExcludes: REQUIRED_SENSITIVE_EXCLUDES,
    },
  );

  dialog = page.getByTestId("nxtlinq-setup-review-dialog");
  await expect(dialog).not.toContainText(
    "Waiting for the Agent's new Desktop draft",
  );
  await expect(
    dialog.getByTestId("nxtlinq-manifest-policy-editor"),
  ).toBeVisible();
  await expect(dialog.getByTestId("nxtlinq-workspace-step")).toHaveCount(0);
  await expect(dialog).toContainText("Regenerated narrow source-only proposal");
  await expect(
    dialog.getByLabel("Editable Nxtlinq permission proposal"),
  ).toHaveValue(/regenerated-policy/);
  await expect(
    dialog.getByLabel(
      "I reviewed the current and proposed manifest shown above.",
    ),
  ).not.toBeChecked();
});
