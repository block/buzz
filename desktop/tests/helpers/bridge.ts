import type { Page } from "@playwright/test";
import { FEATURE_OVERRIDES_STORAGE_KEY, PREVIEW_FEATURE_IDS } from "./features";

export const TEST_IDENTITIES = {
  tyler: {
    privateKey:
      "3dbaebadb5dfd777ff25149ee230d907a15a9e1294b40b830661e65bb42f6c03",
    pubkey: "e5ebc6cdb579be112e336cc319b5989b4bb6af11786ea90dbe52b5f08d741b34",
    username: "tyler",
  },
  alice: {
    privateKey:
      "3fa69cbac1dcb9b7b6ac83117c74bd23bb1e717fe8fc7cfda67b47bb4323383d",
    pubkey: "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f",
    username: "alice",
  },
  bob: {
    privateKey:
      "7667ae87cbc50ac0b2251b115c9c51aca7e2da65301b28ecf82f4e4c5260a6bb",
    pubkey: "bb22a5299220cad76ffd46190ccbeede8ab5dc260faa28b6e5a2cb31b9aff260",
    username: "bob",
  },
  charlie: {
    privateKey:
      "813fc3bb90587a82b2bfee9b833503e7686c7480681850b3d789c6987e997fc8",
    pubkey: "554cef57437abac34522ac2c9f0490d685b72c80478cf9f7ed6f9570ee8624ea",
    username: "charlie",
  },
  outsider: {
    privateKey:
      "91bd673543195c0c78fc74a881545dcc8cd6ea6d0f9f8efb3225d58c4bc70dad",
    pubkey: "df8e91b86fda13a9a67896df77232f7bdab2ba9c3e165378e1ba3d24c13a328e",
    username: "outsider",
  },
} as const;

type BridgeMode = "mock" | "relay";

type MockCommandAvailability = {
  available?: boolean;
  command?: string;
  resolvedPath?: string | null;
};

type MockManagedAgentSeed = {
  pubkey: string;
  name: string;
  personaId?: string | null;
  status?: "running" | "stopped" | "deployed" | "not_deployed";
  channelNames?: string[];
  channelIds?: string[];
  backend?:
    | { type: "local" }
    | { type: "provider"; id: string; config: Record<string, unknown> };
  respondTo?: "owner-only" | "allowlist" | "anyone";
  respondToAllowlist?: string[];
};

type MockSearchProfileSeed = {
  pubkey: string;
  displayName: string | null;
  avatarUrl?: string | null;
  nip05Handle?: string | null;
  about?: string | null;
  ownerPubkey?: string | null;
  isAgent?: boolean;
};

type MockRelayAgentSeed = {
  pubkey: string;
  name: string;
  respondTo?: "owner-only" | "allowlist" | "anyone";
  respondToAllowlist?: string[];
  channelNames?: string[];
  channelIds?: string[];
  status?: "online" | "away" | "offline";
};

export type MockEngramEntry = {
  slug: string;
  body: string;
  eventId: string;
  createdAt: number;
  outgoingRefs: string[];
};

export type MockAgentMemoryListing = {
  core: MockEngramEntry | null;
  memories: MockEngramEntry[];
  truncated: boolean;
  fetchedAt: number;
};

type MockBridgeOptions = {
  acpRuntimesCatalog?: Record<string, unknown>[];
  activePersonaIds?: string[];
  /**
   * Listing returned by the mocked `get_agent_memory` command. Pass a single
   * listing for any managed agent, or a pubkey-keyed record for per-agent data.
   */
  agentMemory?: MockAgentMemoryListing | Record<string, MockAgentMemoryListing>;
  managedAgentPrereqs?: {
    acp?: MockCommandAvailability;
    mcp?: MockCommandAvailability;
  };
  managedAgents?: MockManagedAgentSeed[];
  relayAgents?: MockRelayAgentSeed[];
  agentListDelayMs?: number;
  createManagedAgentDelayMs?: number;
  channelsReadError?: string;
  feedReadError?: string;
  canvasReadError?: string;
  openDmDelayMs?: number;
  sendMessageDelayMs?: number;
  usersBatchDelayMs?: number;
  /** Delay (ms) for older-history fetches; see e2eBridge mock config. */
  historyDelayMs?: number;
  profileReadDelayMs?: number;
  profileReadError?: string;
  profileUpdateError?: string;
  profileUpdateErrors?: string[];
  searchProfiles?: MockSearchProfileSeed[];
  updateAvailable?: boolean;
  updateChannelDelayMs?: number;
  updateDownloadDelayMs?: number;
  updateVersion?: string;
  stallWebsocketSends?: boolean;
  userSearchDelayMs?: number;
  // NIP-IA gate inputs — drive the archive-button gate matrix in
  // tests/e2e/identity-archive.spec.ts.
  /**
   * Lowercase-hex pubkeys returned by `list_archived_identities`. Drives the
   * "Archived on this relay" flair + Unarchive button.
   */
  archivedIdentities?: string[];
  /**
   * Drives the `is_me` field of `resolve_oa_owner`. When true, the harness
   * reports the active identity as the verified NIP-OA owner of the viewee
   * (owner-path branch of the gate).
   */
  oaOwnerIsMe?: boolean;
  /**
   * Active identity's role in the seeded `mockRelayMembers`. `null` removes
   * the active identity from the membership list entirely (admin-path branch
   * evaluates false).
   */
  relayRole?: "owner" | "admin" | "member" | null;
  /**
   * Reporter pubkey injected into mocked mesh serve targets. Defaults to the
   * active identity; specs can override to catch malformed/missing #p handling.
   */
  meshReporterPubkey?: string;
  /**
   * Descriptors returned by the mocked `pick_and_upload_media` /
   * `upload_media_bytes` commands. When omitted, the bridge returns a single
   * generic PDF so the file-attachment flow can be exercised by default. An
   * explicit `[]` is honoured (models a picker cancel / no files selected).
   */
  uploadDelayMs?: number;
  uploadDescriptors?: {
    url: string;
    sha256: string;
    size: number;
    type: string;
    uploaded: number;
    dim?: string;
    blurhash?: string;
    thumb?: string;
    duration?: number;
    image?: string;
    filename?: string;
  }[];
};

type BridgeOptions = {
  mode: BridgeMode;
  mock?: MockBridgeOptions;
  relayHttpUrl?: string;
  relayWsUrl?: string;
  skipOnboardingSeed?: boolean;
  skipWorkspaceSeed?: boolean;
  /**
   * When true (default), seed every preview feature in preview-features.json as
   * enabled in localStorage so E2E tests can interact with gated UI without
   * clicking through the Experiments settings panel. Set to false in specs
   * that exercise the Experiments toggle UI itself.
   */
  seedPreviewFeatures?: boolean;
  user?: keyof typeof TEST_IDENTITIES;
};

const WELCOME_CHANNEL_ENSURED_STORAGE_KEY_PREFIX =
  "buzz-welcome-channel-ensured.v2:";
const ONBOARDING_COMPLETION_STORAGE_KEY_PREFIX = "buzz-onboarding-complete.v1:";
const DEFAULT_MOCK_PUBKEY = "deadbeef".repeat(8);
const DEFAULT_RELAY_WS_URL = "ws://localhost:3000";

function cloneEngramEntry(entry: MockEngramEntry): MockEngramEntry {
  return {
    ...entry,
    outgoingRefs: [...entry.outgoingRefs],
  };
}

/**
 * Recreates the old Memories UI development fixture as explicit Playwright
 * seed data. Use with `installMockBridge(page, { agentMemory: ... })`.
 */
export function createMockAgentMemoryListing(
  overrides: Partial<MockAgentMemoryListing> = {},
): MockAgentMemoryListing {
  const listing: MockAgentMemoryListing = {
    core: {
      slug: "core",
      body: `I am a mock agent used to flesh out the Memories panel.

I prefer concise updates, explicit next steps, and visual polish before edge-case handling.

See [[mem/preferences/ui-density]] and [[mem/projects/buzz-memory-viewer]] for details.

A retired launch checklist used to live at [[mem/archive/deleted-launch-checklist]], but that memory was deleted after the plan changed.`,
      eventId: "mock-core",
      createdAt: 1_700_000_000,
      outgoingRefs: [
        "mem/preferences/ui-density",
        "mem/projects/buzz-memory-viewer",
        "mem/archive/deleted-launch-checklist",
      ],
    },
    memories: [
      {
        slug: "mem/preferences/ui-density",
        body: "Prefer compact lists with generous body text when expanded.\n\nNested ref: [[mem/working-style/review-loop]]",
        eventId: "mock-ui-density",
        createdAt: 1_700_000_100,
        outgoingRefs: ["mem/working-style/review-loop"],
      },
      {
        slug: "mem/working-style/review-loop",
        body: "Ship small slices, screenshot the happy path, then iterate on empty/error states.",
        eventId: "mock-review-loop",
        createdAt: 1_700_000_200,
        outgoingRefs: [],
      },
      {
        slug: "mem/projects/buzz-memory-viewer",
        body: "Building the IXI-7 read-only memory viewer in the profile panel.\n\nChild memory: [[mem/projects/buzz-memory-viewer/notes]]",
        eventId: "mock-project",
        createdAt: 1_700_000_300,
        outgoingRefs: ["mem/projects/buzz-memory-viewer/notes"],
      },
      {
        slug: "mem/projects/buzz-memory-viewer/notes",
        body: "Tree should auto-expand core. Everything else collapsed with a one-line preview.",
        eventId: "mock-project-notes",
        createdAt: 1_700_000_400,
        outgoingRefs: [],
      },
      {
        slug: "mem/people/alice",
        body: "Alice prefers async updates in #design.",
        eventId: "mock-alice",
        createdAt: 1_700_000_500,
        outgoingRefs: [],
      },
      {
        slug: "mem/people/bob",
        body: "Bob reviews PRs quickly but wants screenshots.",
        eventId: "mock-bob",
        createdAt: 1_700_000_600,
        outgoingRefs: ["mem/people/alice"],
      },
      {
        slug: "mem/scratch/todo",
        body: "",
        eventId: "mock-empty",
        createdAt: 1_700_000_700,
        outgoingRefs: [],
      },
      {
        slug: "mem/orphan/unreferenced",
        body: "This orphaned note is not reachable from core. It still points at [[mem/research/old-panel-sketches]], a deleted design scratchpad from an earlier pass.",
        eventId: "mock-orphan",
        createdAt: 1_700_000_800,
        outgoingRefs: ["mem/research/old-panel-sketches"],
      },
    ],
    truncated: true,
    fetchedAt: Math.floor(Date.now() / 1000),
  };

  return {
    core:
      overrides.core === undefined
        ? listing.core
          ? cloneEngramEntry(listing.core)
          : null
        : overrides.core
          ? cloneEngramEntry(overrides.core)
          : null,
    memories: (overrides.memories ?? listing.memories).map(cloneEngramEntry),
    truncated: overrides.truncated ?? listing.truncated,
    fetchedAt: overrides.fetchedAt ?? listing.fetchedAt,
  };
}

async function seedOnboardingCompletionForKnownIdentities(
  page: Page,
  relayWsUrl?: string,
) {
  const pubkeys = [
    DEFAULT_MOCK_PUBKEY,
    ...Object.values(TEST_IDENTITIES).map(({ pubkey }) => pubkey),
  ];
  await page.addInitScript(
    ({ onboardingPrefix, pubkeys: pubkeysToSeed, relayUrl, welcomePrefix }) => {
      const welcomeScope = encodeURIComponent(relayUrl);
      for (const pubkey of pubkeysToSeed) {
        window.localStorage.setItem(`${onboardingPrefix}${pubkey}`, "true");
        window.localStorage.setItem(
          `${welcomePrefix}${welcomeScope}:${pubkey}`,
          "true",
        );
      }
    },
    {
      onboardingPrefix: ONBOARDING_COMPLETION_STORAGE_KEY_PREFIX,
      pubkeys,
      relayUrl: relayWsUrl ?? DEFAULT_RELAY_WS_URL,
      welcomePrefix: WELCOME_CHANNEL_ENSURED_STORAGE_KEY_PREFIX,
    },
  );
}

async function seedDefaultWorkspace(page: Page, relayWsUrl?: string) {
  await page.addInitScript(
    ({ relayUrl }) => {
      const workspaceId = "e2e-default-workspace";
      const workspace = {
        id: workspaceId,
        name: "E2E Test",
        relayUrl,
        addedAt: new Date().toISOString(),
      };
      window.localStorage.setItem(
        "buzz-workspaces",
        JSON.stringify([workspace]),
      );
      window.localStorage.setItem("buzz-active-workspace-id", workspaceId);
    },
    { relayUrl: relayWsUrl ?? DEFAULT_RELAY_WS_URL },
  );
}

async function seedPreviewFeaturesEnabled(page: Page) {
  await page.addInitScript(
    ({ key, ids }) => {
      const overrides: Record<string, boolean> = {};
      for (const id of ids) overrides[id] = true;
      window.localStorage.setItem(key, JSON.stringify(overrides));
    },
    { key: FEATURE_OVERRIDES_STORAGE_KEY, ids: PREVIEW_FEATURE_IDS },
  );
}

export async function installBridge(page: Page, options: BridgeOptions) {
  const identity =
    options.mode === "relay"
      ? TEST_IDENTITIES[options.user ?? "tyler"]
      : undefined;

  // Most specs seed a workspace so useWorkspaceInit doesn't show WelcomeSetup.
  // skipOnboardingSeed only controls the onboarding-completion flag.
  if (!options.skipWorkspaceSeed) {
    await seedDefaultWorkspace(page, options.relayWsUrl);
  }
  if (!options.skipOnboardingSeed) {
    await seedOnboardingCompletionForKnownIdentities(page, options.relayWsUrl);
  }
  // Default to opting every preview feature in. Specs that exercise the
  // Experiments toggle UI itself pass `seedPreviewFeatures: false`.
  if (options.seedPreviewFeatures !== false) {
    await seedPreviewFeaturesEnabled(page);
  }

  await page.addInitScript(
    ({ identity: bridgeIdentity, mock, mode, relayHttpUrl, relayWsUrl }) => {
      const notificationLog: Array<{
        body: string | null;
        title: string;
      }> = [];
      const notificationInstances: MockNotification[] = [];

      class MockNotification extends EventTarget {
        static permission: NotificationPermission = "granted";

        static async requestPermission(): Promise<NotificationPermission> {
          return MockNotification.permission;
        }

        body: string | null;
        onclick: ((event: Event) => void) | null = null;
        title: string;

        constructor(title: string, options?: NotificationOptions) {
          super();
          this.title = title;
          this.body = options?.body ?? null;
          notificationInstances.push(this);
          notificationLog.push({
            body: this.body,
            title: this.title,
          });
        }

        close() {}
      }

      Object.defineProperty(window, "Notification", {
        configurable: true,
        value: MockNotification,
        writable: true,
      });

      const testWindow = window as Window & {
        __BUZZ_E2E__?: Record<string, unknown>;
        __BUZZ_E2E_APP_BADGE_COUNT__?: number;
        __BUZZ_E2E_APP_BADGE_STATE__?: string;
        __BUZZ_E2E_CLICK_NOTIFICATION__?: (index: number) => boolean;
        __BUZZ_E2E_NOTIFICATIONS__?: Array<{
          body: string | null;
          title: string;
        }>;
      };
      const currentConfig = testWindow.__BUZZ_E2E__ ?? {};

      testWindow.__BUZZ_E2E__ = {
        ...currentConfig,
        identity: bridgeIdentity ?? currentConfig.identity,
        mock,
        mode,
        relayHttpUrl: relayHttpUrl ?? currentConfig.relayHttpUrl,
        relayWsUrl: relayWsUrl ?? currentConfig.relayWsUrl,
      };
      testWindow.__BUZZ_E2E_APP_BADGE_COUNT__ = 0;
      testWindow.__BUZZ_E2E_APP_BADGE_STATE__ = "none";
      testWindow.__BUZZ_E2E_CLICK_NOTIFICATION__ = (index: number) => {
        const notification = notificationInstances[index];
        if (!notification) {
          return false;
        }

        const event = new Event("click");
        notification.dispatchEvent(event);
        notification.onclick?.(event);
        return true;
      };
      testWindow.__BUZZ_E2E_NOTIFICATIONS__ = notificationLog;
    },
    {
      identity,
      mock: options.mock,
      mode: options.mode,
      relayHttpUrl: options.relayHttpUrl,
      relayWsUrl: options.relayWsUrl,
    },
  );
}

export async function installMockBridge(
  page: Page,
  mock?: MockBridgeOptions,
  options?: {
    relayWsUrl?: string;
    skipOnboardingSeed?: boolean;
    skipWorkspaceSeed?: boolean;
    seedPreviewFeatures?: boolean;
  },
) {
  await installBridge(page, {
    mode: "mock",
    mock,
    relayWsUrl: options?.relayWsUrl,
    skipOnboardingSeed: options?.skipOnboardingSeed,
    skipWorkspaceSeed: options?.skipWorkspaceSeed,
    seedPreviewFeatures: options?.seedPreviewFeatures,
  });
}

export async function installRelayBridge(
  page: Page,
  user: keyof typeof TEST_IDENTITIES = "tyler",
  options?: { seedPreviewFeatures?: boolean },
) {
  await installBridge(page, {
    mode: "relay",
    user,
    seedPreviewFeatures: options?.seedPreviewFeatures,
  });
}

// The sidebar no longer renders a "browse channels" icon button; the channel
// browser is opened via the primary-modifier + Shift + O keyboard shortcut.
export async function openChannelBrowser(page: Page) {
  await page.getByTestId("app-sidebar").waitFor({ state: "visible" });
  const isMacBrowser = await page.evaluate(() =>
    /mac|iphone|ipad|ipod/i.test(navigator.platform),
  );
  await page.evaluate((isMac) => {
    window.dispatchEvent(
      new KeyboardEvent("keydown", {
        bubbles: true,
        cancelable: true,
        ctrlKey: !isMac,
        key: "O",
        metaKey: isMac,
        shiftKey: true,
      }),
    );
  }, isMacBrowser);
}
