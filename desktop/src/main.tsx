import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "@/app/App";
import "@fontsource-variable/inter/wght.css";
import "@/shared/styles/globals.css";
import { UpdaterProvider } from "@/features/settings/hooks/UpdaterProvider";
import { migrateLegacyWorkspaceStorageBeforeRender } from "@/features/workspaces/legacyWorkspaceStorage";
import { WorkspacesProvider } from "@/features/workspaces/useWorkspaces";
import { ThemeProvider } from "@/shared/theme/ThemeProvider";
import { EmojiBurstProvider } from "@/shared/ui/EmojiBurstProvider";
import { PoofBurstProvider } from "@/shared/ui/PoofBurstProvider";
import { Toaster } from "@/shared/ui/sonner";
import { TooltipProvider } from "@/shared/ui/tooltip";

type E2eWindow = Window & {
  __BUZZ_E2E__?: unknown;
};

const E2E_DEFAULT_PUBKEY = "deadbeef".repeat(8);
const E2E_WORKSPACE_ID = "e2e-default-workspace";
const ONBOARDING_COMPLETION_STORAGE_KEY_PREFIX =
  "sprout-onboarding-complete.v1:";

function configureDevE2eBridgeFromUrl() {
  if (!import.meta.env.DEV) {
    return;
  }

  const url = new URL(window.location.href);
  if (url.searchParams.get("e2e") !== "mock") {
    return;
  }

  const e2eWindow = window as E2eWindow;
  e2eWindow.__BUZZ_E2E__ ??= { mode: "mock" };

  const workspace = {
    addedAt: new Date().toISOString(),
    id: E2E_WORKSPACE_ID,
    name: "E2E Test",
    relayUrl: "ws://localhost:3000",
  };
  window.localStorage.setItem("sprout-workspaces", JSON.stringify([workspace]));
  window.localStorage.setItem("sprout-active-workspace-id", E2E_WORKSPACE_ID);
  window.localStorage.setItem(
    `${ONBOARDING_COMPLETION_STORAGE_KEY_PREFIX}${E2E_DEFAULT_PUBKEY}`,
    "true",
  );
}

function renderApp() {
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <WorkspacesProvider>
        <ThemeProvider defaultTheme="houston">
          <TooltipProvider delayDuration={300}>
            <EmojiBurstProvider>
              <PoofBurstProvider>
                <UpdaterProvider>
                  <App />
                </UpdaterProvider>
                <Toaster />
              </PoofBurstProvider>
            </EmojiBurstProvider>
          </TooltipProvider>
        </ThemeProvider>
      </WorkspacesProvider>
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
  configureDevE2eBridgeFromUrl();
  await installE2eBridgeIfConfigured();
  await migrateLegacyWorkspaceStorageBeforeRender();
  renderApp();
}

void bootstrap();
