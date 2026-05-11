import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "@/app/App";
import "@/shared/styles/globals.css";
import { UpdaterProvider } from "@/features/settings/hooks/UpdaterProvider";
import { WorkspacesProvider } from "@/features/workspaces/useWorkspaces";
import { ThemeProvider } from "@/shared/theme/ThemeProvider";
import { Toaster } from "@/shared/ui/sonner";
import { TooltipProvider } from "@/shared/ui/tooltip";

type E2eWindow = Window & {
  __SPROUT_E2E__?: unknown;
};

function renderApp() {
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <WorkspacesProvider>
        <ThemeProvider defaultTheme="houston">
          <TooltipProvider delayDuration={300}>
            <UpdaterProvider>
              <App />
            </UpdaterProvider>
            <Toaster />
          </TooltipProvider>
        </ThemeProvider>
      </WorkspacesProvider>
    </React.StrictMode>,
  );
}

async function installE2eBridgeIfConfigured() {
  // Keep the large E2E bridge out of the normal startup path and production
  // bundle; only load it when tests explicitly inject an E2E config.
  if (!(window as E2eWindow).__SPROUT_E2E__) {
    return;
  }

  const { maybeInstallE2eTauriMocks } = await import("@/testing/e2eBridge");
  maybeInstallE2eTauriMocks();
}

async function bootstrap() {
  await installE2eBridgeIfConfigured();
  renderApp();
}

void bootstrap();
