import "@fontsource-variable/manrope/index.css";
import "@/shared/styles/globals.css";

import * as React from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import { ThemeProvider } from "@/shared/theme/ThemeProvider";
import { AdvancedSignIn } from "./AdvancedSignIn";
import { InviteSampleScene } from "./InviteScenes";
import { NativeUnavailableScreen } from "./NativeUnavailableScreen";
import {
  OnboardingScenePresentation,
  type OnboardingSceneData,
} from "./OnboardingScenePresentation";
import {
  ONBOARDING_SCENE_IDS,
  type OnboardingSceneId,
} from "./onboardingScenes";

const params = new URLSearchParams(window.location.search);
const queryClient = new QueryClient();
const requestedScene = params.get("scene");
const requestedReskin = params.get("reskin");
const scene: OnboardingSceneId = ONBOARDING_SCENE_IDS.includes(
  requestedScene as OnboardingSceneId,
)
  ? (requestedScene as OnboardingSceneId)
  : "account";

const data: OnboardingSceneData = {
  name: scene === "additional" ? "" : "Alex Example",
  email: scene === "account" ? "" : "alex@example.com",
  signedInEmail: "alex@example.com",
  business: scene === "additional" ? "" : "Sample Business",
  website: "",
  description:
    scene === "additional"
      ? ""
      : "We run social media for independent shops, mostly around Johannesburg.",
  harnessLabel: scene.startsWith("openrouter")
    ? "OpenCode"
    : scene.startsWith("credits") ||
        scene === "funding" ||
        scene.startsWith("api-")
      ? "Colony Agent"
      : "Claude Code",
  harnessStatus:
    scene.startsWith("credits") ||
    scene === "funding" ||
    scene.startsWith("api-")
      ? "Included"
      : scene === "subscription-missing"
        ? "Not installed"
        : "Installed",
  connectionLabel: "Claude Code · this Mac",
  visualOnly: true,
};

createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ThemeProvider>
      <QueryClientProvider client={queryClient}>
        {requestedReskin === "native-unavailable" ? (
          <NativeUnavailableScreen
            body="Colony could not connect to the native startup bridge. Nothing was sent to the relay, and no fallback identity was created. Check the desktop host and try again."
            details="Native startup bridge: timed out after 10s"
            onBack={() => undefined}
            onRetry={() => undefined}
            title="Colony could not start"
          />
        ) : requestedReskin === "invite" ? (
          <InviteSampleScene />
        ) : requestedReskin === "advanced" ? (
          <AdvancedSignIn
            error={null}
            hasSelectedIdentity={false}
            identityLost={false}
            isPending={false}
            onBack={() => undefined}
            onCreate={() => undefined}
            onImport={async () => undefined}
            onRecovered={async () => undefined}
          />
        ) : (
          <OnboardingScenePresentation
            data={data}
            onAdvanced={() => undefined}
            onGoogleSignIn={() => undefined}
            onHaveInviteLink={() => undefined}
            onSignOut={() => undefined}
            scene={scene}
          />
        )}
      </QueryClientProvider>
    </ThemeProvider>
  </React.StrictMode>,
);
