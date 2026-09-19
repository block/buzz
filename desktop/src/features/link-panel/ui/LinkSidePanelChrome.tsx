import { useLocation } from "@tanstack/react-router";
import {
  AppWindow,
  ArrowLeft,
  ArrowRight,
  Camera,
  Copy,
  Inspect,
  RefreshCw,
} from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { deriveShellRoute } from "@/app/AppShell.helpers";
import {
  closePinWebviewInspect,
  getPinWebviewNavState,
  inspectPinWebview,
  PIN_WEBVIEW_RESTORE_EVENT,
  pinWebviewGoBack,
  pinWebviewGoForward,
  pinWebviewReload,
  screenshotPinWebview,
  subscribePinWebviewNav,
  type PinWebviewNavState,
} from "@/features/pinned-sites/lib/pinWebview";
import {
  playgroundConversationFromRoute,
  playgroundScreenshotAvailable,
} from "@/features/playground/lib/conversation";
import {
  playgroundScreenshotFile,
  stagePlaygroundScreenshotDraft,
} from "@/features/playground/lib/screenshot";
import {
  openPopoutWindow,
  popoutErrorMessage,
} from "@/features/popout/lib/popoutWindow";
import { copyTextToClipboard } from "@/shared/lib/clipboard";
import { Button, type ButtonProps } from "@/shared/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/shared/ui/tooltip";

import {
  closeLinkSidePanel,
  getLinkSidePanel,
  setLinkSidePanelViewportMode,
  type LinkSidePanelViewportMode,
} from "../lib/linkSidePanelStore";

function ChromeIconButton({
  tooltip,
  ...props
}: ButtonProps & { tooltip: string }) {
  const button = <Button {...props} />;
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        {props.disabled ? <span className="inline-flex">{button}</span> : button}
      </TooltipTrigger>
      <TooltipContent side="bottom">{tooltip}</TooltipContent>
    </Tooltip>
  );
}

function ModeButton({
  active,
  label,
  onSelect,
  testId,
}: {
  active: boolean;
  label: string;
  onSelect: () => void;
  testId: string;
}) {
  return (
    <button
      className={
        active
          ? "rounded-md bg-secondary px-2 py-0.5 text-xs"
          : "rounded-md px-2 py-0.5 text-xs text-muted-foreground hover:text-foreground"
      }
      data-testid={testId}
      onClick={onSelect}
      type="button"
    >
      {label}
    </button>
  );
}

/**
 * Playground-style web chrome for the link/pin slide-out (and detached OS
 * window). Title + close stay on IdleAuxiliaryPanel when embedded.
 *
 * Layout (two rows so long URLs cannot push tooling off-screen):
 * 1. URL only — truncates/ellipsis within the available header width
 * 2. Desktop/Responsive/Mobile on the left; nav/tool icons right-justified
 *
 * Detach opens the existing pop-out window path (`kind: "link"`). Inspect is
 * only shown once the browser is detached into its own window.
 */
export function LinkSidePanelChrome({
  detached = false,
  pinId,
  url,
  viewportMode,
  onViewportModeChange,
}: {
  /** True when this chrome is hosted in a link pop-out OS/embedded window. */
  detached?: boolean;
  pinId: string;
  url: string;
  viewportMode: LinkSidePanelViewportMode;
  onViewportModeChange?: (mode: LinkSidePanelViewportMode) => void;
}) {
  const location = useLocation();
  const conversation = React.useMemo(() => {
    const route = deriveShellRoute(location.pathname);
    const search = location.search as {
      thread?: unknown;
      threadRootId?: unknown;
    };
    const thread = search.threadRootId ?? search.thread;
    return playgroundConversationFromRoute({
      selectedView: route.selectedView,
      selectedChannelId: route.selectedChannelId,
      threadId: typeof thread === "string" ? thread : null,
    });
  }, [location.pathname, location.search]);
  const canScreenshot = playgroundScreenshotAvailable(conversation);
  const setViewportMode = onViewportModeChange ?? setLinkSidePanelViewportMode;

  const [nav, setNav] = React.useState<PinWebviewNavState>({
    canGoBack: false,
    canGoForward: false,
    currentUrl: url,
  });
  const [detachBusy, setDetachBusy] = React.useState(false);

  React.useEffect(() => {
    let cancelled = false;
    void getPinWebviewNavState(pinId).then((state) => {
      if (!cancelled) {
        setNav({
          canGoBack: state.canGoBack,
          canGoForward: state.canGoForward,
          currentUrl: state.currentUrl || url,
        });
      }
    });
    const unlisten = subscribePinWebviewNav((payload) => {
      if (payload.pinId !== pinId) return;
      setNav({
        canGoBack: payload.canGoBack,
        canGoForward: payload.canGoForward,
        currentUrl: payload.currentUrl || url,
      });
    });
    return () => {
      cancelled = true;
      void unlisten.then((stop) => stop());
    };
  }, [pinId, url]);

  // Leaving the detached window closes inspect with the pin webview teardown.
  // While embedded, Inspect is hidden — clear any stray inspector just in case.
  React.useEffect(() => {
    if (detached) return;
    void closePinWebviewInspect(pinId).catch(() => {});
  }, [detached, pinId]);

  const currentUrl = nav.currentUrl || url;

  async function handleInspect() {
    if (!detached) return;
    try {
      await inspectPinWebview(pinId);
      window.dispatchEvent(new Event(PIN_WEBVIEW_RESTORE_EVENT));
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Could not open inspector.",
      );
    }
  }

  async function handleDetach() {
    if (detached || detachBusy) return;
    const panel = getLinkSidePanel();
    const title = panel?.title?.trim() || "Link";
    const keepAlive = panel?.keepAlive ?? false;
    setDetachBusy(true);
    try {
      await openPopoutWindow({
        kind: "link",
        title,
        seed: pinId,
        link: {
          url: currentUrl || url,
          pinId,
          viewportMode,
          keepAlive,
        },
      });
      // Main slide-out yields to the OS/embedded window (same pattern as
      // playground pop-out dismissing the in-main overlay).
      closeLinkSidePanel();
    } catch (error) {
      toast.error(popoutErrorMessage(error, "Could not detach browser."));
    } finally {
      setDetachBusy(false);
    }
  }

  async function handleScreenshot() {
    if (!conversation) return;
    try {
      const result = await screenshotPinWebview(pinId);
      const file = playgroundScreenshotFile(result);
      stagePlaygroundScreenshotDraft({ conversation, file });
      toast.success("Screenshot added to draft");
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Could not capture page.",
      );
    }
  }

  return (
    <TooltipProvider>
      <div
        className="flex min-w-0 max-w-full flex-col items-stretch gap-0.5"
        data-testid="link-side-panel-chrome"
      >
        {/*
         * Row 1: URL only (title/X live on IdleAuxiliaryPanel). Truncate so
         * long addresses never shove chrome controls off-screen.
         */}
        <div
          className="min-w-0 truncate rounded-md border border-border bg-muted/40 px-2 py-0.5 font-mono text-2xs text-muted-foreground"
          data-testid="link-side-panel-url"
          title={currentUrl}
        >
          {currentUrl}
        </div>
        {/*
         * Row 2: device modes on the left, tooling icons right-justified.
         */}
        <div
          className="flex min-w-0 items-center gap-1"
          data-testid="link-side-panel-mode-row"
        >
          <div className="flex min-w-0 items-center gap-1">
            <ModeButton
              active={viewportMode === "desktop"}
              label="Desktop"
              onSelect={() => setViewportMode("desktop")}
              testId="link-side-panel-mode-desktop"
            />
            <ModeButton
              active={viewportMode === "responsive"}
              label="Responsive"
              onSelect={() => setViewportMode("responsive")}
              testId="link-side-panel-mode-responsive"
            />
            <ModeButton
              active={viewportMode === "mobile"}
              label="Mobile"
              onSelect={() => setViewportMode("mobile")}
              testId="link-side-panel-mode-mobile"
            />
          </div>
          <div
            className="ml-auto flex shrink-0 items-center gap-0.5"
            data-testid="link-side-panel-tool-icons"
          >
            <ChromeIconButton
              aria-label="Back"
              data-testid="link-side-panel-back"
              disabled={!nav.canGoBack}
              onClick={() => {
                void pinWebviewGoBack(pinId).then(setNav);
              }}
              size="icon-xs"
              tooltip="Back"
              type="button"
              variant="ghost"
            >
              <ArrowLeft />
            </ChromeIconButton>
            <ChromeIconButton
              aria-label="Forward"
              data-testid="link-side-panel-forward"
              disabled={!nav.canGoForward}
              onClick={() => {
                void pinWebviewGoForward(pinId).then(setNav);
              }}
              size="icon-xs"
              tooltip="Forward"
              type="button"
              variant="ghost"
            >
              <ArrowRight />
            </ChromeIconButton>
            <ChromeIconButton
              aria-label="Refresh"
              data-testid="link-side-panel-refresh"
              onClick={() => {
                void pinWebviewReload(pinId);
              }}
              size="icon-xs"
              tooltip="Refresh"
              type="button"
              variant="ghost"
            >
              <RefreshCw />
            </ChromeIconButton>
            <ChromeIconButton
              aria-label="Copy URL"
              data-testid="link-side-panel-copy-url"
              onClick={() => copyTextToClipboard(currentUrl, "URL copied")}
              size="icon-xs"
              tooltip="Copy URL"
              type="button"
              variant="ghost"
            >
              <Copy />
            </ChromeIconButton>
            {detached ? (
              <ChromeIconButton
                aria-label="Inspect"
                data-testid="link-side-panel-inspect"
                onClick={() => void handleInspect()}
                size="icon-xs"
                tooltip="Inspect"
                type="button"
                variant="outline"
              >
                <Inspect />
              </ChromeIconButton>
            ) : null}
            {canScreenshot ? (
              <ChromeIconButton
                aria-label="Screenshot"
                data-testid="link-side-panel-screenshot"
                onClick={() => void handleScreenshot()}
                size="icon-xs"
                tooltip="Screenshot"
                type="button"
                variant="outline"
              >
                <Camera />
              </ChromeIconButton>
            ) : null}
            {detached ? null : (
              <ChromeIconButton
                aria-label="Detach"
                data-testid="link-side-panel-detach"
                disabled={detachBusy}
                onClick={() => void handleDetach()}
                size="icon-xs"
                tooltip="Detach"
                type="button"
                variant="outline"
              >
                <AppWindow />
              </ChromeIconButton>
            )}
          </div>
        </div>
      </div>
    </TooltipProvider>
  );
}
