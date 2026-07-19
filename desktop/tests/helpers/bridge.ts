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
  lastError?: string | null;
  lastErrorCode?: number | null;
  needsRestart?: boolean;
  autoRestartOnConfigChange?: boolean;
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
  agentType?: string;
  capabilities?: string[];
  respondTo?: "owner-only" | "allowlist" | "anyone";
  respondToAllowlist?: string[];
  channelNames?: string[];
  channelIds?: string[];
  status?: "online" | "away" | "offline";
};

type MockPersonaSeed = {
  id?: string;
  displayName: string;
  avatarUrl?: string | null;
  systemPrompt: string;
  isActive?: boolean;
  sourceTeam?: string | null;
  envVars?: Record<string, string>;
  /**
   * Runtime the persona is pinned to (e.g. "goose", "codex", "claude"). Lets a
   * spec seed a CLI-login runtime whose provider picker is hidden, so the Edit
   * dialog's provider-aware submit gate can be driven end-to-end. Omitted →
   * null (definition inherits the app default at open).
   */
  runtime?: string | null;
  /** Model pinned on the persona (a custom model id for Customize mode). */
  model?: string | null;
  /** Provider pinned on the persona. Leave empty for Codex/Claude runtimes. */
  provider?: string | null;
  namePool?: string[];
};

type MockTeamSeed = {
  id?: string;
  name: string;
  description?: string | null;
  personaIds: string[];
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
  acpRuntimesDelayMs?: number;
  acpAuthMethods?: Record<string, { methods: Record<string, unknown>[] }>;
  acpAuthMethodsError?: string;
  connectAcpRuntimeResult?: { launched: boolean };
  connectAcpRuntimeDelayMs?: number;
  connectAcpRuntimeError?: string;
  /** Override the result returned by the `install_acp_runtime` mock command.
   *  Pass `{ success: false, steps: [...] }` to exercise error/Retry states. */
  installAcpRuntimeResult?: {
    success: boolean;
    steps: {
      step: string;
      command: string;
      success: boolean;
      stdout: string;
      stderr: string;
      exit_code: number | null;
      hint?: string;
    }[];
  };
  /** Sequence of results for successive `install_acp_runtime` calls. Call N
   *  returns results[N]; when exhausted the last entry repeats. Takes precedence
   *  over `installAcpRuntimeResult`. Use for fail-then-succeed Retry tests. */
  installAcpRuntimeResults?: Array<{
    success: boolean;
    steps: {
      step: string;
      command: string;
      success: boolean;
      stdout: string;
      stderr: string;
      exit_code: number | null;
      hint?: string;
    }[];
  }>;
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
  personas?: MockPersonaSeed[];
  teams?: MockTeamSeed[];
  relayAgents?: MockRelayAgentSeed[];
  agentListDelayMs?: number;
  createManagedAgentDelayMs?: number;
  addChannelMembersDelayMs?: number;
  /** Sequenced add-member failures. A string fails that call; null succeeds. */
  addChannelMembersErrors?: (string | null)[];
  channelMembersReadDelayMs?: number;
  channelsReadError?: string;
  /** Reject successive mock `create_channel` calls, then resume. */
  createChannelErrors?: string[];
  /** Reject successive mock `ensure_starter_channels` calls, then resume. */
  ensureStarterChannelsErrors?: string[];
  /** Reject successive mock `join_channel` calls, then resume. */
  joinChannelErrors?: string[];
  /** Number of seeded rows in the deep-history fixture. Defaults to 600. */
  deepHistoryMessageCount?: number;
  feedReadError?: string;
  canvasReadError?: string;
  /** Delay (ms) for `apply_workspace`; see e2eBridge mock config. */
  applyCommunityDelayMs?: number;
  openDmDelayMs?: number;
  sendMessageDelayMs?: number;
  /** Close the first channel-window live REQ; its retry is accepted. */
  closeChannelLiveSubscriptionOnce?: boolean;
  /** Reject successive kind-9 sends with these messages, then resume. */
  sendMessageErrors?: string[];
  /** Reject successive managed-agent starts, then resume. */
  startManagedAgentErrors?: string[];
  /** Delay (ms) after snapshotting a thread-replies page so E2E tests can
   * deliver live reply/aux events while an older response is in flight. */
  threadRepliesDelayMs?: number;
  usersBatchDelayMs?: number;
  /** Delay (ms) for older-history fetches; see e2eBridge mock config. */
  channelWindowDelayMs?: number;
  profileReadDelayMs?: number;
  profileReadError?: string;
  profileUpdateError?: string;
  profileUpdateErrors?: string[];
  searchProfiles?: MockSearchProfileSeed[];
  updateAvailable?: boolean;
  updateChannelDelayMs?: number;
  updateDownloadDelayMs?: number;
  updateVersion?: string;
  /** Set to false to simulate a Linux .deb install where auto-update is not
   *  supported. Defaults to true. See e2eBridge mock.autoUpdateSupported. */
  autoUpdateSupported?: boolean;
  /** Reject browser opener calls to exercise manual pairing fallback UI. */
  openerError?: string;
  /** Delay binding signatures so specs can exercise request supersession. */
  nostrBindSignDelayMs?: number;
  stallWebsocketSends?: boolean;
  userSearchDelayMs?: number;
  /**
   * Value returned by the `observer_archive_default_enabled` mock command.
   * `true` = internal-policy build (toggle locked ON); `false`/omitted = OSS
   * build (toggle functional). Drives LocalArchiveSettingsCard policy state.
   */
  observerArchiveDefaultEnabled?: boolean;
  /**
   * Delay (ms) applied to `observer_archive_default_enabled` so specs can
   * assert the pending-reconciliation state (toggle disabled, no
   * `list_save_subscriptions` call yet) before the policy resolves.
   */
  observerArchiveDefaultEnabledDelayMs?: number;
  /**
   * When set, `observer_archive_default_enabled` throws with this message —
   * drives the fail-closed path when the policy check itself fails.
   */
  observerArchiveDefaultEnabledError?: string;
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
   * Descriptors returned by the mocked `pick_and_upload_media` /
   * `upload_media_bytes` commands. When omitted, the bridge returns a single
   * generic PDF so the file-attachment flow can be exercised by default. An
   * explicit `[]` is honoured (models a picker cancel / no files selected).
   */
  uploadDelayMs?: number;
  /** Delay (ms) applied to `encode_agent_snapshot_for_send` so E2E tests can
   *  observe the "preparing" phase before the upload begins. 0/undefined = instant. */
  encodeDelayMs?: number;
  /** Delay (ms) applied to `get_relay_self` so E2E tests can prove the
   *  fail-closed race: DMs are withheld while classification is unresolved. */
  relaySelfDelayMs?: number;
  /**
   * Sequenced results for `confirm_team_snapshot_import`. String = throw
   * with that message; null = succeed. Call N uses results[N]; last entry
   * repeats when exhausted. Follows the `nsecErrors` precedent.
   */
  teamSnapshotConfirmErrors?: (string | null)[];
  /**
   * When true, `preview_team_snapshot_import` returns a preview with
   * `hasSourceAllowlist: true` so the allowlist section renders in the
   * import dialog.
   */
  teamSnapshotPreviewHasSourceAllowlist?: boolean;
  /**
   * When set to a non-empty string, `fetch_snapshot_bytes` throws with this
   * message — lets specs prove malformed/hash/size-mismatch error paths.
   */
  snapshotFetchError?: string;
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
  /**
   * Seed rows returned by the mocked `list_save_subscriptions` command.
   * Drives the LocalArchiveSettingsCard subscription list view in screenshot
   * and UI tests without a real SQLite backend.
   */
  saveSubscriptions?: Array<{
    scope_type: string;
    scope_value: string;
    kinds: string; // JSON-encoded integer array, e.g. "[9,40002]"
  }>;
  /**
   * Event IDs that `get_event` should report as definitively not found.
   * Causes `useDraftRootStatus` to map the draft to `deleted` state so specs
   * can exercise the "Thread deleted" label / disabled-send path.
   */
  deletedEventIds?: string[];
  /**
   * When true, `get_identity` returns `lost: true` until `persist_current_identity`
   * or `import_identity` is invoked. Drives the identity-lost recovery UX in tests.
   */
  identityLost?: boolean;
  /**
   * When true, `get_identity` returns `locked: true` until `import_identity` is
   * invoked. Drives the keyring-locked screen in tests.
   */
  identityLocked?: boolean;
  /**
   * Pending community deep links seeded into the mocked Rust-side queue.
   * The frontend drains these on boot into onboarding or an editable Add
   * Community prefill.
   */
  pendingCommunityDeepLinks?: Array<{
    id: string;
    kind: "connect" | "join" | "add-community";
    relayUrl: string;
    code?: string | null;
    name?: string | null;
  }>;
  /**
   * Global agent config returned by `get_global_agent_config`. Defaults to
   * an empty config (no provider, model, or env vars) if not specified.
   * Pass a config with a provider to test Inherit-from-global behavior.
   */
  globalAgentConfig?: {
    env_vars: Record<string, string>;
    provider: string | null;
    model: string | null;
  };
  bakedBuildEnv?: Array<{
    key: string;
    masked: boolean;
    value: string;
  }>;
  /** Delay (ms) for `get_baked_build_env` so specs can verify load gating. */
  bakedBuildEnvDelayMs?: number;
  /** Delay (ms) for `set_global_agent_config` — hold saves open in tests.
   *  Alias of `globalConfigSaveDelayMs` (kept for onboarding specs). */
  setGlobalAgentConfigDelayMs?: number;
  /**
   * When set, `get_nsec` throws with this message. For a single always-fail
   * scenario. Use `nsecErrors` for sequenced fail/succeed.
   */
  nsecError?: string;
  /**
   * Sequenced results for `get_nsec`. String = throw with that message;
   * null = succeed. Call N uses results[N]; last entry repeats when exhausted.
   */
  nsecErrors?: (string | null)[];
  /**
   * The `restarted_count` returned by `set_global_agent_config`. Defaults to
   * 0 (no agents restarted). Set to a positive integer to drive the
   * "Saved. Restarted N agent(s)." status text in GlobalAgentConfigSettingsCard.
   */
  globalConfigRestartedCount?: number;
  /**
   * The `failed_restart_count` returned by `set_global_agent_config`. Defaults
   * to 0. Set to a positive integer to drive the "failed to restart — check
   * the Agents tab." status text in GlobalAgentConfigSettingsCard.
   */
  globalConfigFailedRestartCount?: number;
  /**
   * Milliseconds to delay the mocked `set_global_agent_config` response.
   * Defaults to 0 (resolve immediately). Use to hold a save in flight so a
   * test can interleave edits and exercise the mid-save race handling.
   */
  globalConfigSaveDelayMs?: number;
};

type BridgeOptions = {
  mode: BridgeMode;
  mock?: MockBridgeOptions;
  relayHttpUrl?: string;
  relayWsUrl?: string;
  skipOnboardingSeed?: boolean;
  skipCommunitySeed?: boolean;
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
// The relay HTTP/WS URLs follow BUZZ_E2E_RELAY_URL (same env var seed.ts reads),
// so a suite pointed at an isolated relay (e.g. the read-model harness on :3030)
// uses it without per-spec wiring. Falls back to the shared dev relay when unset.
const DEFAULT_RELAY_HTTP_URL =
  process.env.BUZZ_E2E_RELAY_URL ?? "http://localhost:3000";
const DEFAULT_RELAY_WS_URL = DEFAULT_RELAY_HTTP_URL.replace(/^http/, "ws");

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

async function seedDefaultCommunity(page: Page, relayWsUrl?: string) {
  await page.addInitScript(
    ({ relayUrl }) => {
      const communityId = "e2e-default-community";
      const community = {
        id: communityId,
        name: "E2E Test",
        relayUrl,
        addedAt: new Date().toISOString(),
      };
      window.localStorage.setItem(
        "buzz-communities",
        JSON.stringify([community]),
      );
      window.localStorage.setItem("buzz-active-community-id", communityId);
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

  // Most specs seed a community so useCommunityInit doesn't show WelcomeSetup.
  // skipOnboardingSeed only controls the onboarding-completion flag.
  if (!options.skipCommunitySeed) {
    await seedDefaultCommunity(page, options.relayWsUrl);
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
    skipCommunitySeed?: boolean;
    seedPreviewFeatures?: boolean;
  },
) {
  await installBridge(page, {
    mode: "mock",
    mock,
    relayWsUrl: options?.relayWsUrl,
    skipOnboardingSeed: options?.skipOnboardingSeed,
    skipCommunitySeed: options?.skipCommunitySeed,
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
    // Thread BUZZ_E2E_RELAY_URL into BOTH transports. The app defaults these to
    // :3000 in relay mode; without explicit wiring HTTP queries (channel list,
    // feed) miss an isolated relay and surface as "Failed to fetch".
    relayHttpUrl: DEFAULT_RELAY_HTTP_URL,
    relayWsUrl: DEFAULT_RELAY_WS_URL,
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

// Section header actions (create channel, new DM, mark all read, sort) now
// live inside a per-section "more actions" (⋮) menu instead of standalone
// header icon buttons. These helpers open that menu and pick an item.
async function openSectionMenu(page: Page, actionsTestId: string) {
  const trigger = page.getByTestId(actionsTestId);
  await trigger.scrollIntoViewIfNeeded();
  await trigger.click();
}

// The Channels section "+" now opens the unified Add-channel browser, so the
// standalone create dialog is reached via the primary-modifier + Shift + N
// keyboard shortcut (the "New channel" menu item was removed as redundant).
export async function openCreateChannelDialog(page: Page) {
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
        key: "N",
        metaKey: isMac,
        shiftKey: true,
      }),
    );
  }, isMacBrowser);
  await page.getByTestId("create-channel-dialog").waitFor();
}

export async function openNewMessagePage(page: Page) {
  await openSectionMenu(page, "section-actions-dms");
  await page.getByRole("menuitem", { name: "New message" }).click();
  await page.getByTestId("new-message-page").waitFor({ state: "visible" });
}
