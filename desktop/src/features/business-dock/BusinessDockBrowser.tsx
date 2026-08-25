import {
  ExternalLink,
  KeyRound,
  LoaderCircle,
  LogOut,
  RotateCw,
} from "lucide-react";

import { useBusinessDock } from "@/features/business-dock/BusinessDockProvider";
import { Button } from "@/shared/ui/button";

export function BusinessDockBrowser() {
  const {
    businessAuth,
    bindCurrentDevice,
    checkBusinessAuth,
    config,
    configError,
    embedSessionPhase,
    iframeRef,
    logoutBusiness,
    onBrowserLoad,
    startBusinessSignIn,
    state,
    ssoMode,
    workbenchAuthPhase,
    workbenchGatewayStatus,
    workbenchGroupClaimStatus,
  } = useBusinessDock();

  if (!config) {
    return (
      <div
        className="flex min-h-0 flex-1 items-center justify-center px-8 text-center text-sm text-muted-foreground"
        data-testid="business-dock-unconfigured"
      >
        {configError ?? "Business system is not configured."}
      </div>
    );
  }

  const needsBusinessAuth =
    businessAuth.phase === "unconnected" ||
    businessAuth.phase === "expired" ||
    businessAuth.phase === "failed";

  return (
    <div className="relative min-h-0 flex-1 bg-background">
      <iframe
        ref={iframeRef}
        allow="clipboard-read; clipboard-write"
        className="h-full w-full border-0 bg-background"
        data-testid="business-dock-iframe"
        onLoad={onBrowserLoad}
        referrerPolicy="strict-origin-when-cross-origin"
        sandbox="allow-downloads allow-forms allow-modals allow-popups allow-popups-to-escape-sandbox allow-same-origin allow-scripts"
        src={config.homeUrl}
        tabIndex={state.open ? 0 : -1}
        title={state.title ?? "Business system"}
      />
      {state.loading ? (
        <div
          className="pointer-events-none absolute inset-0 flex items-center justify-center bg-background/60"
          data-testid="business-dock-loading"
        >
          <LoaderCircle className="size-5 animate-spin text-muted-foreground" />
          <span
            className={
              state.openingResource
                ? "ml-2 text-sm text-muted-foreground"
                : "sr-only"
            }
          >
            {state.openingResource
              ? "正在打开业务页面…"
              : "Loading business system"}
          </span>
        </div>
      ) : null}
      {needsBusinessAuth ? (
        <div
          className="absolute inset-x-4 bottom-4 rounded-lg border bg-background/95 p-4 shadow-lg backdrop-blur"
          data-testid="business-auth-required"
        >
          <p className="text-sm font-medium">
            {businessAuth.phase === "expired"
              ? "Business session expired"
              : businessAuth.phase === "failed"
                ? "Business sign-in failed"
                : "Business sign-in required"}
          </p>
          <p className="mt-1 text-xs text-muted-foreground">
            {businessAuth.reason ??
              "Sign in in your browser, then check the embedded session again."}
          </p>
          <div className="mt-3 flex gap-2">
            {workbenchGatewayStatus === "binding_required" ||
            workbenchGatewayStatus === "device_revoked" ? (
              <Button onClick={bindCurrentDevice} size="sm">
                <KeyRound /> Bind current device
              </Button>
            ) : (
              <Button onClick={startBusinessSignIn} size="sm">
                <ExternalLink /> Continue SSO
              </Button>
            )}
            <Button onClick={checkBusinessAuth} size="sm" variant="outline">
              <RotateCw /> Check again
            </Button>
          </div>
        </div>
      ) : null}
      {import.meta.env.DEV ? (
        <div
          className={`absolute left-3 flex max-w-[calc(100%-1.5rem)] items-center gap-2 rounded-md border bg-background/90 px-3 py-2 text-xs shadow-sm ${needsBusinessAuth ? "bottom-36" : "bottom-3"}`}
          data-testid="business-auth-debug"
        >
          <span>
            Workbench: {workbenchAuthPhase}/{workbenchGatewayStatus} · Business:{" "}
            {businessAuth.phase} · SSO: {ssoMode} · Embed: {embedSessionPhase} ·
            Bridge: Auth V3
            {workbenchGroupClaimStatus
              ? ` · Workbench groups: ${workbenchGroupClaimStatus}`
              : ""}
            {businessAuth.identity
              ? ` · ${businessAuth.identity.displayName}`
              : ""}
          </span>
          {businessAuth.phase === "authenticated" ? (
            <Button
              aria-label="Log out of Business"
              className="size-6"
              onClick={logoutBusiness}
              size="icon"
              variant="ghost"
            >
              <LogOut />
            </Button>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
