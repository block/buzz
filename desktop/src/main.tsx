import React from "react";
import ReactDOM from "react-dom/client";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { App } from "@/app/App";
import { RootErrorBoundary } from "@/app/RootErrorBoundary";
import { NostrBindConsentDialog } from "@/features/profile/ui/NostrBindConsentDialog";
import "@fontsource-variable/inter/opsz.css";
import "@fontsource-variable/inter/opsz-italic.css";
import "@fontsource/jetbrains-mono/400.css";
import "@fontsource/jetbrains-mono/700.css";
import "@/shared/styles/globals.css";
import { UpdaterProvider } from "@/features/settings/hooks/UpdaterProvider";
import { migrateLegacyCommunityStorageBeforeRender } from "@/features/communities/legacyCommunityStorage";
import { CommunitiesProvider } from "@/features/communities/useCommunities";
import { huddleWindowChannelId } from "@/features/huddle/lib/huddleWindow";
import { CommunityOnboardingProvider } from "@/features/onboarding/communityOnboarding";
import { ThemeProvider } from "@/shared/theme/ThemeProvider";
import { EmojiBurstProvider } from "@/shared/ui/EmojiBurstProvider";
import { PoofBurstProvider } from "@/shared/ui/PoofBurstProvider";
import { Toaster } from "@/shared/ui/sonner";
import { TooltipProvider } from "@/shared/ui/tooltip";
import { recoverLocalStorageQuotaOnStartup } from "@/shared/lib/localStorageQuota";
import { startLocalStorageSweep } from "@/shared/lib/localStorageSweep";
import { initializeConversationDensityPreference } from "@/shared/lib/conversationDensityPreference";
import { initializeFontSizePreference } from "@/shared/lib/fontSizePreference";

type E2eWindow = Window & {
  __BUZZ_E2E__?: unknown;
};

const E2E_DEFAULT_PUBKEY = "deadbeef".repeat(8);
const E2E_COMMUNITY_ID = "e2e-default-community";
const ONBOARDING_COMPLETION_STORAGE_KEY_PREFIX = "buzz-onboarding-complete.v1:";
const DEV_STATE_RESET_PARAM = "resetDevState";

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

/**
 * Seeds the same default mock community/identity state
 * `configureDevE2eBridgeFromUrl`'s `?e2e=mock` path seeds, without requiring
 * that URL param. `configureDevE2eBridgeFromUrl` itself only ever runs under
 * `import.meta.env.DEV`, so an e2e-mode build (`pnpm build:e2e`, used by the
 * native-smoke launch) never reaches it — a native-smoke caller must seed
 * through this function directly instead.
 */
function seedDefaultE2eConfig() {
  const e2eWindow = window as E2eWindow;
  e2eWindow.__BUZZ_E2E__ ??= { mode: "mock" };

  const community = {
    addedAt: new Date().toISOString(),
    id: E2E_COMMUNITY_ID,
    name: "E2E Test",
    relayUrl: "ws://localhost:3000",
  };
  window.localStorage.setItem("buzz-communities", JSON.stringify([community]));
  window.localStorage.setItem("buzz-active-community-id", E2E_COMMUNITY_ID);
  window.localStorage.setItem(
    `${ONBOARDING_COMPLETION_STORAGE_KEY_PREFIX}${E2E_DEFAULT_PUBKEY}`,
    "true",
  );
}

function configureDevE2eBridgeFromUrl() {
  if (!import.meta.env.DEV) {
    return;
  }

  const url = new URL(window.location.href);
  if (url.searchParams.get("e2e") !== "mock") {
    return;
  }

  seedDefaultE2eConfig();
}

function renderApp() {
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      {/* block/buzz#5078 — catch any uncaught render error so a WebKit
          SecurityError from localStorage can't blank the whole window. */}
      <RootErrorBoundary>
        <CommunitiesProvider>
          <CommunityOnboardingProvider
            enabled={huddleWindowChannelId() === null}
          >
            <ThemeProvider defaultTheme="buzz">
              <TooltipProvider>
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
      </RootErrorBoundary>
    </React.StrictMode>,
  );
}

async function isPluginBrowserSmokeEnabled(): Promise<boolean> {
  // `BUZZ_PLUGIN_BROWSER_SMOKE` is a Rust process variable the bundle cannot
  // read directly (`import.meta.env` is build-time), so this asks the native
  // command instead. Only ever reachable under DEV/e2e with native internals
  // present — a production frontend build never calls this.
  if (!isTauri()) {
    return false;
  }
  try {
    return await invoke<boolean>("plugin_browser_smoke_enabled");
  } catch {
    return false;
  }
}

async function installE2eBridgeIfConfigured(nativeSmoke: boolean) {
  // The mock bridge is compiled only into dev and explicit E2E builds. A
  // pre-bootstrap global alone must never activate mock IPC in production.
  if (!(import.meta.env.DEV || import.meta.env.MODE === "e2e")) {
    return;
  }

  if (!nativeSmoke && !(window as E2eWindow).__BUZZ_E2E__) {
    return;
  }

  const { maybeInstallE2eTauriMocks } = await import("@/testing/e2eBridge");
  maybeInstallE2eTauriMocks({ nativeSmoke });

  if (nativeSmoke) {
    // Side-effect import: the driver walks the real UI as soon as it loads.
    // This import is only ever reachable behind the native-smoke check above.
    await import("@/features/plugins/smokeDriver.e2e.ts");
  }
}

async function bootstrap() {
  resetDevWebviewStateFromUrl();
  configureDevE2eBridgeFromUrl();
  recoverLocalStorageQuotaOnStartup();
  initializeConversationDensityPreference();
  initializeFontSizePreference();
  startLocalStorageSweep();
  // Determined once, ahead of `installE2eBridgeIfConfigured`: a native-smoke
  // launch needs its own default mock community/identity state seeded before
  // that function's config check, since it reaches this build (`pnpm
  // build:e2e`, `MODE=e2e`) without the DEV-only `?e2e=mock` URL path that
  // seeds it for ordinary dev/e2e runs. Gated the same way that function
  // gates itself: a production build must never probe a native command for
  // this on every startup.
  const nativeSmoke =
    import.meta.env.DEV || import.meta.env.MODE === "e2e"
      ? await isPluginBrowserSmokeEnabled()
      : false;
  if (nativeSmoke && !(window as E2eWindow).__BUZZ_E2E__) {
    seedDefaultE2eConfig();
  }
  await installE2eBridgeIfConfigured(nativeSmoke);
  await migrateLegacyCommunityStorageBeforeRender();
  renderApp();
}

void bootstrap();
