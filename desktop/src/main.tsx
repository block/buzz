import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "@/app/App";
import { NostrBindConsentDialog } from "@/features/profile/ui/NostrBindConsentDialog";
import "@fontsource-variable/inter/wght.css";
import "@/shared/styles/globals.css";
import { UpdaterProvider } from "@/features/settings/hooks/UpdaterProvider";
import { migrateLegacyCommunityStorageBeforeRender } from "@/features/communities/legacyCommunityStorage";
import { CommunitiesProvider } from "@/features/communities/useCommunities";
import { CommunityOnboardingProvider } from "@/features/onboarding/communityOnboarding";
import {
  ANNOUNCEMENT_DEMO_AGENTS,
  ANNOUNCEMENT_DEMO_COMMUNITY_NAME,
  ANNOUNCEMENT_DEMO_INITIAL_READ_CHANNEL_IDS,
  ANNOUNCEMENT_DEMO_PEOPLE,
  ANNOUNCEMENT_DEMO_SECTION_STORE,
} from "@/testing/announcementDemoFixtures";
import { localReadStateKey } from "@/features/channels/readState/readStateFormat";
import { ThemeProvider } from "@/shared/theme/ThemeProvider";
import { EmojiBurstProvider } from "@/shared/ui/EmojiBurstProvider";
import { PoofBurstProvider } from "@/shared/ui/PoofBurstProvider";
import { Toaster } from "@/shared/ui/sonner";
import { TooltipProvider } from "@/shared/ui/tooltip";

type E2eWindow = Window & {
  __BUZZ_E2E__?: unknown;
};

const E2E_DEFAULT_PUBKEY = "deadbeef".repeat(8);
const E2E_COMMUNITY_ID = "e2e-default-community";
const ONBOARDING_COMPLETION_STORAGE_KEY_PREFIX = "buzz-onboarding-complete.v1:";
const DEV_STATE_RESET_PARAM = "resetDevState";
const ANNOUNCEMENT_DEMO_QUERY_VALUE = "announcement";
const CHANNEL_SECTIONS_STORAGE_KEY_PREFIX = "buzz-channel-sections.v1";
const SELF_PROFILE_STORAGE_KEY_PREFIX = "buzz-self-profile.v1";

function resetDevWebviewStateFromUrl() {
  if (!import.meta.env.DEV) {
    return;
  }

  const url = new URL(window.location.href);
  if (url.searchParams.get(DEV_STATE_RESET_PARAM) !== "1") {
    return;
  }

  // WebKit groups every Buzz binary under one disk directory, but storage is
  // isolated by origin. Clearing here resets only this dev server's origin;
  // deleting the shared WebKit directory would also destroy installed-app state.
  window.localStorage.clear();
  window.sessionStorage.clear();
  url.searchParams.delete(DEV_STATE_RESET_PARAM);
  window.history.replaceState(window.history.state, "", url);
}

function configureMockBridgeFromUrl() {
  const url = new URL(window.location.href);
  const isDevE2eMock =
    import.meta.env.DEV && url.searchParams.get("e2e") === "mock";
  const isAnnouncementDemo =
    url.searchParams.get("demo") === ANNOUNCEMENT_DEMO_QUERY_VALUE ||
    import.meta.env.VITE_ANNOUNCEMENT_DEMO === "1";
  const mockRelayWsUrl = isAnnouncementDemo
    ? `${url.protocol === "https:" ? "wss:" : "ws:"}//${url.host}`
    : "ws://localhost:3000";

  if (!isDevE2eMock && !isAnnouncementDemo) {
    return;
  }

  // The native recording build intentionally reuses the browser mock bridge;
  // only live model calls leave the webview through the local provider proxy.
  const e2eWindow = window as E2eWindow;
  if (isAnnouncementDemo) {
    e2eWindow.__BUZZ_E2E__ = {
      mode: "mock",
      relayHttpUrl: url.origin,
      relayWsUrl: mockRelayWsUrl,
      mock: {
        announcementDemo: true,
        managedAgents: ANNOUNCEMENT_DEMO_AGENTS.map((agent) => ({
          pubkey: agent.pubkey,
          name: agent.name,
          avatarUrl: agent.avatarUrl,
          systemPrompt: agent.systemPrompt,
          status: "running" as const,
          channelNames: [...agent.channelNames],
          respondTo: "owner-only" as const,
        })),
      },
    };
  } else {
    e2eWindow.__BUZZ_E2E__ ??= { mode: "mock" };
  }

  const community = {
    addedAt: new Date().toISOString(),
    id: E2E_COMMUNITY_ID,
    name: isAnnouncementDemo ? ANNOUNCEMENT_DEMO_COMMUNITY_NAME : "E2E Test",
    relayUrl: mockRelayWsUrl,
  };
  window.localStorage.setItem("buzz-communities", JSON.stringify([community]));
  window.localStorage.setItem("buzz-active-community-id", E2E_COMMUNITY_ID);
  window.localStorage.setItem(
    `${ONBOARDING_COMPLETION_STORAGE_KEY_PREFIX}${E2E_DEFAULT_PUBKEY}`,
    "true",
  );

  if (isAnnouncementDemo) {
    const relayStorageScope = encodeURIComponent(mockRelayWsUrl);
    const initialReadAt = new Date().toISOString();
    window.localStorage.setItem(
      `${CHANNEL_SECTIONS_STORAGE_KEY_PREFIX}:${E2E_DEFAULT_PUBKEY}:${relayStorageScope}`,
      JSON.stringify(ANNOUNCEMENT_DEMO_SECTION_STORE),
    );
    window.localStorage.setItem(
      localReadStateKey(E2E_DEFAULT_PUBKEY),
      JSON.stringify(
        Object.fromEntries(
          ANNOUNCEMENT_DEMO_INITIAL_READ_CHANNEL_IDS.map((channelId) => [
            channelId,
            initialReadAt,
          ]),
        ),
      ),
    );
    window.localStorage.setItem(
      `${SELF_PROFILE_STORAGE_KEY_PREFIX}:${mockRelayWsUrl}:${E2E_DEFAULT_PUBKEY}`,
      JSON.stringify({
        version: 1,
        displayName: ANNOUNCEMENT_DEMO_PEOPLE.viewer.displayName,
        avatarUrl: ANNOUNCEMENT_DEMO_PEOPLE.viewer.avatarUrl,
        avatarDataUrl: null,
        updatedAt: Date.now(),
        hasProfileEvent: true,
      }),
    );
  }
}

function renderApp() {
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <CommunitiesProvider>
        <CommunityOnboardingProvider>
          <ThemeProvider defaultTheme="buzz">
            <TooltipProvider delayDuration={300}>
              <EmojiBurstProvider>
                <PoofBurstProvider>
                  <UpdaterProvider>
                    <App />
                    <NostrBindConsentDialog />
                  </UpdaterProvider>
                  <Toaster />
                </PoofBurstProvider>
              </EmojiBurstProvider>
            </TooltipProvider>
          </ThemeProvider>
        </CommunityOnboardingProvider>
      </CommunitiesProvider>
    </React.StrictMode>,
  );
}

async function installE2eBridgeIfConfigured() {
  // Keep the large E2E bridge out of the normal startup path and production
  // bundle; only load it when tests explicitly inject an E2E config.
  if (!(window as E2eWindow).__BUZZ_E2E__) {
    return;
  }

  const { maybeInstallE2eTauriMocks } = await import("@/testing/e2eBridge");
  maybeInstallE2eTauriMocks();
}

async function bootstrap() {
  resetDevWebviewStateFromUrl();
  configureMockBridgeFromUrl();
  await installE2eBridgeIfConfigured();
  await migrateLegacyCommunityStorageBeforeRender();
  renderApp();
}

void bootstrap();
