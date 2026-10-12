import * as React from "react";
import { isTauri } from "@tauri-apps/api/core";
import { createFileRoute } from "@tanstack/react-router";

import { ViewLoadingFallback } from "@/shared/ui/ViewLoadingFallback";

const BrowsersScreen = React.lazy(async () => {
  const module = await import("@/features/browsers/ui/BrowsersScreen");
  return { default: module.BrowsersScreen };
});

const WebBrowsersScreen = React.lazy(async () => {
  const module = await import("@/features/browsers/ui/WebBrowsersScreen");
  return { default: module.WebBrowsersScreen };
});

export const Route = createFileRoute("/browsers")({
  component: BrowsersRouteComponent,
});

function BrowsersRouteComponent() {
  return (
    <React.Suspense fallback={<ViewLoadingFallback kind="browsers" />}>
      {isTauri() ? <BrowsersScreen /> : <WebBrowsersScreen />}
    </React.Suspense>
  );
}
