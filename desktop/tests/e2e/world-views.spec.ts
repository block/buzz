import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";

import type {
  ResolvedWorldView,
  WorldViewBinding,
  WorldViewBindingsDocument,
} from "../../src/shared/api/worldViewTypes";
import { installMockBridge } from "../helpers/bridge";

const SEEDED_BINDING_ID = "11111111-1111-4111-8111-111111111111";
const ADDED_BINDING_ID = "22222222-2222-4222-8222-222222222222";
const PASTED_BINDING_ID = "33333333-3333-4333-8333-333333333333";
const GENERAL_CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const MESSAGE_THREAD_ROOT_ID = "mock-general-welcome";
const FORUM_THREAD_ROOT_ID = "5".repeat(64);

function hostedBinding(
  id: string,
  label: string,
  displayMode: WorldViewBinding["displayMode"] = "tasks",
): WorldViewBinding {
  return {
    id,
    label,
    reference: {
      kind: "hosted-world-view-export",
      origin: "https://manifest.shivai.space",
      shareToken: "view-token",
    },
    realmQualifiedName: "world::main",
    viewQualifiedName: "@main::Board",
    displayMode,
  };
}
function mutableHostedBinding(id: string, label: string): WorldViewBinding {
  return {
    id,
    label,
    reference: {
      kind: "hosted-world-latest",
      origin: "https://manifest.shivai.space",
      hostedWorldId: "mock-hosted-world",
    },
    realmQualifiedName: "world::main",
    viewQualifiedName: "@main::Board",
    displayMode: "tasks",
  };
}

function resolvedWorldView(
  bindingId: string,
  label: string,
  authorityKind:
    | "default"
    | "hosted-world-latest"
    | "hosted-world-live-view-share" = "default",
): ResolvedWorldView {
  const presentationModel: ResolvedWorldView["presentation"]["dark"] = {
    graph: {
      kind: "ready",
      graphBackgroundHex: "#111113",
      graphPattern: "dots",
      clusters: [],
      nodes: [
        {
          id: `${bindingId}:bind`,
          label: "Bind channel mirror",
          preferenceQualifiedName: "world::main::BindChannelMirror",
          status: "ready",
          targetState: null,
          isReady: true,
          isLeaf: false,
          fillHex: "#1c2024",
          borderHex: "#3e63dd",
          textHex: "#f0f0f3",
          deemphasis: null,
          effect: null,
          position: { x: 150, y: 57.5 },
          size: { width: 300, height: 115 },
        },
        {
          id: `${bindingId}:render`,
          label: "Render shared view",
          preferenceQualifiedName: "world::main::RenderSharedView",
          status: "ready",
          targetState: null,
          isReady: true,
          isLeaf: false,
          fillHex: "#1c2024",
          borderHex: "#8e4ec6",
          textHex: "#f0f0f3",
          deemphasis: null,
          effect: null,
          position: { x: 150, y: 297.5 },
          size: { width: 300, height: 115 },
        },
        {
          id: `${bindingId}:refresh`,
          label: "Refresh after agent turn",
          preferenceQualifiedName: "world::main::RefreshAfterAgentTurn",
          status: "ready",
          targetState: null,
          isReady: true,
          isLeaf: false,
          fillHex: "#1c2024",
          borderHex: "#12a594",
          textHex: "#f0f0f3",
          deemphasis: null,
          effect: null,
          position: { x: 150, y: 537.5 },
          size: { width: 300, height: 115 },
        },
        {
          id: `${bindingId}:ship`,
          label,
          preferenceQualifiedName: "world::main::ShipBuzzIntegration",
          status: "ready",
          targetState: null,
          isReady: true,
          isLeaf: true,
          fillHex: "#1c2024",
          borderHex: "#e5484d",
          textHex: "#f0f0f3",
          deemphasis: null,
          effect: null,
          position: { x: 150, y: 777.5 },
          size: { width: 300, height: 115 },
        },
      ],
      edges: [
        {
          id: `${bindingId}:bind-render`,
          sourceId: "world::main::BindChannelMirror",
          targetId: "world::main::RenderSharedView",
          connectionType: "foundational",
          flowspaceQualifiedName: "%main::Plan",
          lineHex: "#6e6ade",
          deemphasis: null,
        },
        {
          id: `${bindingId}:render-refresh`,
          sourceId: "world::main::RenderSharedView",
          targetId: "world::main::RefreshAfterAgentTurn",
          connectionType: "foundational",
          flowspaceQualifiedName: "%main::Plan",
          lineHex: "#6e6ade",
          deemphasis: null,
        },
        {
          id: `${bindingId}:refresh-ship`,
          sourceId: "world::main::RefreshAfterAgentTurn",
          targetId: "world::main::ShipBuzzIntegration",
          connectionType: "foundational",
          flowspaceQualifiedName: "%main::Plan",
          lineHex: "#6e6ade",
          deemphasis: null,
        },
      ],
      bounds: { width: 420, height: 955 },
    },
    revision: "revision-world-view-1",
    selection: {
      realmQualifiedName: "world::main",
      viewQualifiedName: "@main::Board",
    },
  };
  const viewDumpNodes: ResolvedWorldView["viewDump"]["nodes"] = [
    "Bind channel mirror",
    "Render shared view",
    "Refresh after agent turn",
    label,
  ].map((preference, index) => ({
    preference,
    qualifiedName: `world::main::Task${index + 1}`,
    status: "ready",
    actionable: index === 3,
    leaf: index === 3,
    inFocus: false,
    inSatisfied: false,
    blockers: [],
    enablers: [],
    note: { preview: null, truncated: false },
    signals: [],
  }));

  return {
    formatVersion: 1,
    bindingId,
    channelId: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
    declaredScope: { kind: "channel" },
    effectiveScope: { kind: "channel" },
    bindingRevisionEventId: "1".repeat(64),
    sourceRevision: "revision-world-view-1",
    freshness:
      bindingId === ADDED_BINDING_ID ? "latest-at-resolution" : "pinned",
    authority:
      authorityKind === "hosted-world-latest"
        ? {
            kind: "hosted-world-latest",
            origin: "https://manifest.shivai.space",
            hostedWorldId: "mock-hosted-world",
          }
        : authorityKind === "hosted-world-live-view-share"
          ? {
              kind: "hosted-world-live-view-share",
              origin: "https://manifest.shivai.space",
              hostedWorldId: "mock-hosted-world",
            }
          : bindingId === ADDED_BINDING_ID
            ? {
                kind: "local-world-mirror-latest",
                origin: "https://manifest.shivai.space",
                mirrorId: "mirror-buzz-main",
              }
            : {
                kind: "hosted-world-view-export",
                origin: "https://manifest.shivai.space",
              },
    realm: { name: "main", qualifiedName: "world::main" },
    view: { name: "Board", qualifiedName: "@main::Board" },
    viewDump: {
      counts: {
        nodes: 4,
        edges: 3,
        ready: 4,
        actionableReady: 1,
        satisfied: 0,
        blocked: 0,
      },
      nodes: viewDumpNodes,
      readyLeaves: [viewDumpNodes[3]],
      satisfiedNodes: [],
      blockedNodes: [],
      edges: [
        {
          downstream: "RenderSharedView",
          upstream: "BindChannelMirror",
          relation: "blocker",
          connectionType: "foundational",
          flowspace: "Plan",
          flowspaceQualifiedName: "%main::Plan",
        },
        {
          downstream: "RefreshAfterAgentTurn",
          upstream: "RenderSharedView",
          relation: "blocker",
          connectionType: "foundational",
          flowspace: "Plan",
          flowspaceQualifiedName: "%main::Plan",
        },
        {
          downstream: "ShipBuzzIntegration",
          upstream: "RefreshAfterAgentTurn",
          relation: "blocker",
          connectionType: "foundational",
          flowspace: "Plan",
          flowspaceQualifiedName: "%main::Plan",
        },
      ],
    },
    presentation: {
      formatVersion: 1,
      dark: presentationModel,
      light: {
        ...presentationModel,
        graph: {
          ...presentationModel.graph,
          graphBackgroundHex: "#f9f9fb",
          nodes: presentationModel.graph.nodes.map((node) => ({
            ...node,
            fillHex: "#ffffff",
            textHex: "#1c2024",
          })),
        },
      },
    },
    resolvedAt: "2026-07-24T12:00:00Z",
    nextCommand:
      `buzz world-views resolve --channel ` +
      `aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa --binding ${bindingId}`,
  };
}

async function openGeneralChannel(page: Page) {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
}

test.describe("Shivai world views", () => {
  test("renders a bound task view from the shared Shivai presentation", async ({
    page,
  }) => {
    const consoleErrors: string[] = [];
    page.on("console", (message) => {
      if (message.type() === "error") {
        consoleErrors.push(message.text());
      }
    });
    await installMockBridge(page, {
      worldViewBindings: {
        version: 4,
        scope: { kind: "channel" },
        bindings: [hostedBinding(SEEDED_BINDING_ID, "Launch board")],
      },
      resolvedWorldViews: {
        [SEEDED_BINDING_ID]: resolvedWorldView(
          SEEDED_BINDING_ID,
          "Ship Buzz integration",
        ),
      },
    });

    await openGeneralChannel(page);

    const worldViews = page.getByTestId("channel-world-views");
    await expect(worldViews).toBeVisible();
    await expect(worldViews).toContainText("Shivai world views");
    await expect(worldViews).toContainText("Launch board");
    await expect(worldViews).toContainText("Ship Buzz integration");
    await expect(worldViews).toContainText("Pinned export");
    await expect(worldViews.getByLabel("World view summary")).toContainText(
      "4 ready",
    );
    await expect(
      worldViews.locator('[data-world-view-tile-surface="true"]'),
    ).toHaveCount(1);

    const graphButton = worldViews.getByRole("button", {
      name: "Graph",
      exact: true,
    });
    await graphButton.click();
    await expect(graphButton).toHaveAttribute("aria-pressed", "true");
    const tile = worldViews.locator('[data-world-view-tile-surface="true"]');
    await expect(tile).toHaveAttribute("data-display-mode", "graph");
    const graphCanvas = worldViews.getByTestId("workbench-graph-canvas");
    await expect(graphCanvas).toBeVisible();
    await expect(graphCanvas).toHaveCSS("opacity", "1");
    const viewportHeight = page.viewportSize()?.height;
    expect(viewportHeight).toBeDefined();
    await worldViews
      .getByRole("button", { name: "Expand Shivai world views" })
      .click();
    await expect(worldViews).toHaveAttribute("data-maximized", "true");
    await expect
      .poll(async () => (await worldViews.boundingBox())?.height ?? 0)
      .toBeGreaterThan((viewportHeight ?? 0) * 0.8);
    await worldViews
      .getByRole("button", { name: "Restore Shivai world views" })
      .click();
    await expect(worldViews).toHaveAttribute("data-maximized", "false");
    await expect
      .poll(async () => {
        const renderedNodes = await graphCanvas.getAttribute(
          "data-workbench-rendered-nodes",
        );
        return renderedNodes ? JSON.parse(renderedNodes).length : 0;
      })
      .toBe(4);
    const graphCaptureSurface = worldViews.getByTestId(
      "workbench-graph-capture-surface",
    );
    const fitGraphButton = worldViews.getByRole("button", {
      name: "Fit graph",
    });
    await fitGraphButton.click();
    await expect(graphCaptureSurface).toHaveAttribute(
      "data-workbench-viewport-motion",
      "active",
    );
    await expect(graphCaptureSurface).toHaveAttribute(
      "data-workbench-viewport-motion",
      "idle",
    );
    const initialZoom = Number(
      await graphCanvas.getAttribute("data-workbench-zoom"),
    );
    expect(initialZoom).toBeGreaterThan(0);
    await worldViews.getByRole("button", { name: "Zoom out" }).click();
    await expect
      .poll(async () =>
        Number(await graphCanvas.getAttribute("data-workbench-zoom")),
      )
      .toBeLessThan(initialZoom);
    const zoomedOut = Number(
      await graphCanvas.getAttribute("data-workbench-zoom"),
    );
    await worldViews.getByRole("button", { name: "Zoom in" }).click();
    await expect
      .poll(async () =>
        Number(await graphCanvas.getAttribute("data-workbench-zoom")),
      )
      .toBeGreaterThan(zoomedOut);
    await fitGraphButton.click();
    await expect(graphCanvas).toHaveAttribute(
      "data-workbench-layout-state",
      "idle",
    );
    await expect
      .poll(() =>
        page.evaluate(() =>
          document.fonts.check('700 16px "Averia Serif Libre", serif'),
        ),
      )
      .toBe(true);
    expect(consoleErrors).toEqual([]);

    await expect
      .poll(() =>
        page.evaluate(
          () =>
            window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
              channelName: "general",
              kind: 40101,
            }) ?? false,
        ),
      )
      .toBe(true);
    await page.evaluate(() => {
      const binding = window.__BUZZ_E2E__?.mock?.worldViewBindings?.bindings[0];
      if (!binding) throw new Error("Missing mocked world-view binding");
      binding.label = "Live launch board";
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: "{}",
        kind: 40101,
      });
    });
    await expect(worldViews).toContainText("Live launch board");

    const commands = await page.evaluate(
      () => window.__BUZZ_E2E_COMMANDS__ ?? [],
    );
    expect(commands).toEqual(
      expect.arrayContaining(["get_world_view_bindings", "resolve_world_view"]),
    );
  });

  test("confirms before removing a channel world-view binding", async ({
    page,
  }) => {
    await installMockBridge(page, {
      worldViewBindings: {
        version: 4,
        scope: { kind: "channel" },
        bindings: [hostedBinding(SEEDED_BINDING_ID, "Launch board")],
      },
      resolvedWorldViews: {
        [SEEDED_BINDING_ID]: resolvedWorldView(
          SEEDED_BINDING_ID,
          "Ship Buzz integration",
        ),
      },
    });
    await openGeneralChannel(page);

    const worldViews = page.getByTestId("channel-world-views");
    const removeButton = worldViews.getByRole("button", {
      name: "Remove Launch board",
    });
    await removeButton.click();

    const confirmation = page.getByTestId("world-view-remove-confirmation");
    await expect(confirmation).toBeVisible();
    await expect(confirmation).toContainText(
      "The Shivai world itself will remain unchanged.",
    );
    expect(
      await page.evaluate(
        () =>
          (window.__BUZZ_E2E_COMMANDS__ ?? []).filter(
            (command) => command === "set_world_view_bindings",
          ).length,
      ),
    ).toBe(0);

    await confirmation.getByRole("button", { name: "Keep binding" }).click();
    await expect(confirmation).toBeHidden();
    await expect(worldViews).toContainText("Launch board");

    await removeButton.click();
    await confirmation.getByRole("button", { name: "Remove binding" }).click();
    await expect(confirmation).toBeHidden();
    await expect(
      page.getByRole("button", { name: "Bind a Shivai world view" }),
    ).toBeVisible();

    const commandLog = await page.evaluate(
      () => window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? [],
    );
    expect(
      commandLog.find((entry) => entry.command === "set_world_view_bindings")
        ?.payload,
    ).toMatchObject({
      document: {
        bindings: [],
        scope: { kind: "channel" },
        version: 4,
      },
    });
  });

  test("renders inherited channel state in a message thread and publishes an override", async ({
    page,
  }) => {
    await installMockBridge(page, {
      worldViewBindings: {
        version: 4,
        scope: { kind: "channel" },
        bindings: [hostedBinding(SEEDED_BINDING_ID, "Launch board")],
      },
      resolvedWorldViews: {
        [SEEDED_BINDING_ID]: resolvedWorldView(
          SEEDED_BINDING_ID,
          "Ship Buzz integration",
        ),
      },
    });
    await page.goto(
      `/#/channels/${GENERAL_CHANNEL_ID}?messageId=${MESSAGE_THREAD_ROOT_ID}&thread=${MESSAGE_THREAD_ROOT_ID}`,
    );
    const threadPanel = page.getByTestId("message-thread-panel");
    await expect(threadPanel).toBeVisible();
    const threadViews = threadPanel.getByTestId("channel-world-views");
    await expect(threadViews).toContainText("Inherited from channel");
    await expect(threadViews).toContainText("Launch board");

    await threadViews
      .getByRole("button", { name: "Override Launch board" })
      .click();
    await threadViews.getByLabel("Label").fill("Thread launch board");
    await threadViews.getByRole("button", { name: "Save view" }).click();

    await expect(threadViews).toContainText("Thread override");
    await expect(threadViews).toContainText("Thread launch board");
    const commandLog = await page.evaluate(
      () => window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? [],
    );
    const publishCall = commandLog
      .filter((entry) => entry.command === "set_world_view_bindings")
      .at(-1);
    expect(publishCall?.payload).toMatchObject({
      expectedRevisionEventId: null,
      document: {
        version: 4,
        scope: {
          kind: "thread",
          threadRootEventId: MESSAGE_THREAD_ROOT_ID,
        },
        bindings: [
          {
            id: SEEDED_BINDING_ID,
            label: "Thread launch board",
          },
        ],
      },
    });
  });

  test("renders inherited channel state in the forum thread surface", async ({
    page,
  }) => {
    await installMockBridge(page, {
      worldViewBindings: {
        version: 4,
        scope: { kind: "channel" },
        bindings: [hostedBinding(SEEDED_BINDING_ID, "Forum launch board")],
      },
      resolvedWorldViews: {
        [SEEDED_BINDING_ID]: resolvedWorldView(
          SEEDED_BINDING_ID,
          "Ship forum integration",
        ),
      },
    });
    await page.goto("/");
    await page.getByTestId("channel-watercooler").click();
    await page.evaluate((threadRootEventId) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "watercooler",
        content: "World-view forum thread",
        id: threadRootEventId,
        kind: 45001,
      });
    }, FORUM_THREAD_ROOT_ID);
    await expect(page.getByTestId("chat-title")).toHaveText("watercooler");
    await page.getByText("World-view forum thread", { exact: true }).click();
    const forumThread = page.getByTestId("forum-thread-panel");
    await expect(forumThread).toBeVisible();
    const threadViews = forumThread.getByTestId("channel-world-views");
    await expect(threadViews).toContainText("Inherited from channel");
    await expect(threadViews).toContainText("Forum launch board");
  });

  test("imports a public view link and rejects edit-share capabilities", async ({
    page,
  }) => {
    await installMockBridge(page, {
      resolvedWorldViews: {
        [PASTED_BINDING_ID]: resolvedWorldView(
          PASTED_BINDING_ID,
          "Ship pasted reference",
        ),
      },
    });
    await openGeneralChannel(page);
    await page.evaluate((bindingId) => {
      Object.defineProperty(window.crypto, "randomUUID", {
        configurable: true,
        value: () => bindingId,
      });
    }, PASTED_BINDING_ID);
    await page
      .getByRole("button", { name: "Bind a Shivai world view" })
      .click();

    const worldViews = page.getByTestId("channel-world-views");
    const publicReference = worldViews.getByLabel(
      "Or use a public read-only link",
      { exact: true },
    );
    await publicReference.fill(`Shivai view reference
Source: hosted edit share "edit-secret-token"
Realm: world::main
View qualified: @main::Board`);
    await worldViews
      .getByRole("button", { name: "Use link", exact: true })
      .click();
    await expect(worldViews).toContainText(
      "Edit-share capabilities cannot be published",
    );
    expect(
      await page.evaluate(
        () =>
          (window.__BUZZ_E2E_COMMANDS__ ?? []).filter(
            (command) => command === "set_world_view_bindings",
          ).length,
      ),
    ).toBe(0);

    await publicReference.fill(
      "https://manifest.shivai.space/world/exports/public-view-token",
    );
    await worldViews
      .getByRole("button", { name: "Use link", exact: true })
      .click();
    await expect(worldViews).toContainText("Shared pinned read-only view");
    await expect(worldViews.getByLabel("View", { exact: true })).toHaveValue(
      "@main::Board",
    );
    await worldViews.getByLabel("Label").fill("Pasted launch board");
    await worldViews
      .getByRole("button", { name: "Tasks", exact: true })
      .click();
    await worldViews
      .getByRole("button", { name: "Share in this channel" })
      .click();

    await expect(worldViews).toContainText("Pasted launch board");
    const commandLog = await page.evaluate(
      () => window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? [],
    );
    expect(
      commandLog.find(
        (entry) => entry.command === "connect_local_world_authority",
      ),
    ).toBeUndefined();
    expect(
      commandLog.find((entry) => entry.command === "set_world_view_bindings")
        ?.payload,
    ).toMatchObject({
      document: {
        scope: { kind: "channel" },
        bindings: [
          {
            id: PASTED_BINDING_ID,
            reference: {
              kind: "hosted-world-view-export",
              shareToken: "public-view-token",
            },
            realmQualifiedName: "world::main",
            viewQualifiedName: "@main::Board",
            displayMode: "tasks",
          },
        ],
      },
    });
  });

  test("requires origin trust before resolving an attacker-authored public link", async ({
    page,
  }) => {
    await installMockBridge(page, {
      trustedWorldOrigins: [],
    });
    await openGeneralChannel(page);
    await page
      .getByRole("button", { name: "Bind a Shivai world view" })
      .click();

    const worldViews = page.getByTestId("channel-world-views");
    await worldViews
      .getByLabel("Or use a public read-only link", { exact: true })
      .fill("https://attacker.example/world/exports/attacker-token");
    await worldViews
      .getByRole("button", { name: "Use link", exact: true })
      .click();

    await expect(worldViews).toContainText("Trust this Shivai origin?");
    await expect(worldViews).toContainText(
      "Buzz has not contacted https://attacker.example",
    );
    expect(
      await page.evaluate(
        () =>
          (window.__BUZZ_E2E_COMMANDS__ ?? []).filter(
            (command) => command === "catalog_world_views",
          ).length,
      ),
    ).toBe(0);

    await worldViews
      .getByRole("button", { name: "Trust origin", exact: true })
      .click();
    await expect(worldViews.getByLabel("View", { exact: true })).toHaveValue(
      "@main::Board",
    );

    const commands = await page.evaluate(
      () => window.__BUZZ_E2E_COMMANDS__ ?? [],
    );
    expect(commands.indexOf("trust_world_origin")).toBeGreaterThan(-1);
    expect(commands.indexOf("catalog_world_views")).toBeGreaterThan(
      commands.indexOf("trust_world_origin"),
    );
  });

  test("connects a local world before publishing its mirror binding", async ({
    page,
  }) => {
    await installMockBridge(page, {
      resolvedWorldViews: {
        [ADDED_BINDING_ID]: resolvedWorldView(
          ADDED_BINDING_ID,
          "Verify local mirror binding",
        ),
      },
      worldAuthorities: [
        {
          kind: "local-world-mirror-latest",
          origin: "https://manifest.shivai.space",
          mirrorId: "mirror-buzz-main",
          sourceRoot: "/workspace/buzz-integration.world",
        },
      ],
    });
    await openGeneralChannel(page);
    await page.evaluate((bindingId) => {
      Object.defineProperty(window.crypto, "randomUUID", {
        configurable: true,
        value: () => bindingId,
      });
    }, ADDED_BINDING_ID);

    await page
      .getByRole("button", { name: "Bind a Shivai world view" })
      .click();
    const connectedWorld = page.getByRole("button", {
      name: /buzz-integration\.world/,
    });
    await expect(connectedWorld).toBeVisible();
    await connectedWorld.click();
    await expect(page.getByLabel("View", { exact: true })).toHaveValue(
      "@main::Board",
    );
    await page.getByRole("button", { name: "Connect local world" }).click();
    await expect(page.getByLabel("View", { exact: true })).toHaveValue(
      "@main::Board",
    );
    await page.getByLabel("Label").fill("Local launch board");
    await page.getByRole("button", { name: "Tasks", exact: true }).click();
    await page.getByRole("button", { name: "Share in this channel" }).click();

    const worldViews = page.getByTestId("channel-world-views");
    await expect(worldViews).toContainText("Local launch board");
    await expect(worldViews).toContainText("Verify local mirror binding");
    await expect(worldViews).toContainText("Latest mirror");

    const commandLog = await page.evaluate(
      () => window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? [],
    );
    const authorityCall = commandLog.find(
      (entry) => entry.command === "connect_local_world_authority",
    );
    const publishCall = commandLog.find(
      (entry) => entry.command === "set_world_view_bindings",
    );
    expect(authorityCall?.payload).toEqual({
      sourceRoot: "/workspace/buzz-integration.world",
    });
    expect(publishCall?.payload).toMatchObject({
      expectedRevisionEventId: null,
      document: {
        version: 4,
        scope: { kind: "channel" },
        bindings: [
          {
            id: ADDED_BINDING_ID,
            label: "Local launch board",
            reference: {
              kind: "local-world-mirror-latest",
              origin: "https://manifest.shivai.space",
              mirrorId: "mirror-buzz-main",
            },
            realmQualifiedName: "world::main",
            viewQualifiedName: "@main::Board",
            displayMode: "tasks",
          },
        ],
      },
    });
    const publishedDocument = (
      publishCall?.payload as
        | { document?: WorldViewBindingsDocument }
        | undefined
    )?.document;
    expect(publishedDocument?.bindings[0]?.reference).toEqual({
      kind: "local-world-mirror-latest",
      origin: "https://manifest.shivai.space",
      mirrorId: "mirror-buzz-main",
    });
    expect(
      commandLog.findIndex(
        (entry) => entry.command === "connect_local_world_authority",
      ),
    ).toBeLessThan(
      commandLog.findIndex(
        (entry) => entry.command === "set_world_view_bindings",
      ),
    );
  });
  test("connects a hosted world without publishing its edit-share token", async ({
    page,
  }) => {
    const editShareUrl =
      "https://manifest.shivai.space/world/edit/private-edit-token";
    await installMockBridge(page, {
      resolvedWorldViews: {
        [ADDED_BINDING_ID]: resolvedWorldView(
          ADDED_BINDING_ID,
          "Edit hosted world",
          "hosted-world-live-view-share",
        ),
      },
    });
    await openGeneralChannel(page);
    await page.evaluate((bindingId) => {
      Object.defineProperty(window.crypto, "randomUUID", {
        configurable: true,
        value: () => bindingId,
      });
    }, ADDED_BINDING_ID);

    await page
      .getByRole("button", { name: "Bind a Shivai world view" })
      .click();
    await page.getByRole("button", { name: "Connect hosted world" }).click();
    await page.getByLabel("Hosted edit-share link").fill(editShareUrl);
    await page.getByRole("button", { name: "Connect", exact: true }).click();
    await expect(page.getByLabel("View", { exact: true })).toHaveValue(
      "@main::Board",
    );
    await page.getByLabel("Label").fill("Mutable hosted board");
    await page.getByRole("button", { name: "Share in this channel" }).click();
    const worldViews = page.getByTestId("channel-world-views");
    await expect(worldViews).toContainText("Mutable hosted board");
    await expect(
      worldViews.locator('[data-world-view-tile-surface="true"]'),
    ).toHaveAttribute("data-display-mode", "graph");
    await expect(worldViews).toContainText("Latest shared view");

    const commandLog = await page.evaluate(
      () => window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? [],
    );
    const authorityCall = commandLog.find(
      (entry) => entry.command === "register_hosted_world_authority",
    );
    const publishCall = commandLog.find(
      (entry) => entry.command === "set_world_view_bindings",
    );
    expect(authorityCall?.payload).toEqual({
      credential: editShareUrl,
      origin: "https://manifest.shivai.space",
    });
    expect(publishCall?.payload).toMatchObject({
      expectedRevisionEventId: null,
      document: {
        version: 4,
        scope: { kind: "channel" },
        bindings: [
          {
            id: ADDED_BINDING_ID,
            label: "Mutable hosted board",
            reference: {
              kind: "hosted-world-live-view-share",
              origin: "https://manifest.shivai.space",
              shareToken: "public-live-view-token",
            },
            realmQualifiedName: "world::main",
            viewQualifiedName: "@main::Board",
            displayMode: "graph",
          },
        ],
      },
    });
    expect(JSON.stringify(publishCall?.payload)).not.toContain(
      "private-edit-token",
    );
    const liveShareCall = commandLog.find(
      (entry) => entry.command === "publish_hosted_world_live_view_share",
    );
    expect(liveShareCall?.payload).toEqual({
      reference: {
        kind: "hosted-world-latest",
        origin: "https://manifest.shivai.space",
        hostedWorldId: "mock-hosted-world",
      },
      viewQualifiedName: "@main::Board",
    });
    expect(
      commandLog.findIndex(
        (entry) => entry.command === "publish_hosted_world_live_view_share",
      ),
    ).toBeLessThan(
      commandLog.findIndex(
        (entry) => entry.command === "set_world_view_bindings",
      ),
    );
    expect(
      commandLog.findIndex(
        (entry) => entry.command === "register_hosted_world_authority",
      ),
    ).toBeLessThan(
      commandLog.findIndex(
        (entry) => entry.command === "set_world_view_bindings",
      ),
    );
  });

  test("refreshes an existing hosted source after replacing its authority", async ({
    page,
  }) => {
    await installMockBridge(page, {
      worldAuthorities: [
        {
          kind: "hosted-world-latest",
          origin: "https://manifest.shivai.space",
          hostedWorldId: "mock-hosted-world",
        },
      ],
      worldViewCatalog: {
        formatVersion: 1,
        revision: "catalog-before-registration",
        worldQualifiedName: "world",
        views: [
          {
            name: "@Old",
            qualifiedName: "@main::Old",
            realm: {
              name: "main",
              qualifiedName: "world::main",
            },
          },
        ],
      },
      worldViewCatalogAfterHostedRegistration: {
        formatVersion: 1,
        revision: "catalog-after-registration",
        worldQualifiedName: "world",
        views: [
          {
            name: "@Fresh",
            qualifiedName: "@main::Fresh",
            realm: {
              name: "main",
              qualifiedName: "world::main",
            },
          },
        ],
      },
    });
    await openGeneralChannel(page);

    await page
      .getByRole("button", { name: "Bind a Shivai world view" })
      .click();
    await page.getByRole("button", { name: /Hosted Shivai world/ }).click();
    await expect(page.getByLabel("View", { exact: true })).toHaveValue(
      "@main::Old",
    );

    await page.getByRole("button", { name: "Connect hosted world" }).click();
    await page
      .getByLabel("Hosted edit-share link")
      .fill("https://manifest.shivai.space/world/edit/replacement-token");
    await page.getByRole("button", { name: "Connect", exact: true }).click();

    await expect(page.getByLabel("View", { exact: true })).toHaveValue(
      "@main::Fresh",
    );
  });
  test("enables and revokes agent edits for one exact binding", async ({
    page,
  }) => {
    await installMockBridge(page, {
      worldAuthorities: [
        {
          kind: "hosted-world-latest",
          origin: "https://manifest.shivai.space",
          hostedWorldId: "mock-hosted-world",
        },
      ],
      worldViewBindings: {
        version: 4,
        scope: { kind: "channel" },
        bindings: [
          mutableHostedBinding(SEEDED_BINDING_ID, "Mutable launch board"),
        ],
      },
      resolvedWorldViews: {
        [SEEDED_BINDING_ID]: resolvedWorldView(
          SEEDED_BINDING_ID,
          "Mutable launch board",
          "hosted-world-latest",
        ),
      },
    });
    await openGeneralChannel(page);

    const editSwitch = page.getByRole("switch", {
      name: "Allow agents to edit Mutable launch board",
    });
    await expect(editSwitch).toBeEnabled();
    await expect(editSwitch).toHaveAttribute("aria-checked", "false");

    await editSwitch.click();
    await expect(editSwitch).toHaveAttribute("aria-checked", "true");

    let commandLog = await page.evaluate(
      () => window.__BUZZ_E2E_COMMAND_LOG__ ?? [],
    );
    const authorizeCall = commandLog.find(
      (entry) => entry.command === "authorize_world_view_mutation",
    );
    expect(authorizeCall?.payload).toEqual({
      authority: {
        kind: "hosted-world-latest",
        origin: "https://manifest.shivai.space",
        hostedWorldId: "mock-hosted-world",
      },
      bindingId: SEEDED_BINDING_ID,
      bindingRevisionEventId: "1".repeat(64),
      channelId: GENERAL_CHANNEL_ID,
      declaredScope: { kind: "channel" },
    });

    await editSwitch.click();
    await expect(editSwitch).toHaveAttribute("aria-checked", "false");

    commandLog = await page.evaluate(
      () => window.__BUZZ_E2E_COMMAND_LOG__ ?? [],
    );
    const revokeCall = commandLog.find(
      (entry) => entry.command === "revoke_world_view_mutation",
    );
    expect(revokeCall?.payload).toEqual({
      bindingId: SEEDED_BINDING_ID,
      channelId: GENERAL_CHANNEL_ID,
      declaredScope: { kind: "channel" },
    });
  });
});
