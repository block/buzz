import { useLocation } from "@tanstack/react-router";
import {
  ArrowLeft,
  ArrowRight,
  Camera,
  Copy,
  Inspect,
  Maximize2,
  Minimize2,
  RefreshCw,
} from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { deriveShellRoute } from "@/app/AppShell.helpers";
import {
  getPinWebviewNavState,
  inspectPinWebview,
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
import { copyTextToClipboard } from "@/shared/lib/clipboard";
import { Button, type ButtonProps } from "@/shared/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/shared/ui/tooltip";

import {
  setLinkSidePanelViewportMode,
  toggleLinkSidePanelExpanded,
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
 * Playground-style web chrome for the link/pin slide-out header actions.
 * Title + close stay on IdleAuxiliaryPanel.
 */
export function LinkSidePanelChrome({
  expanded,
  pinId,
  url,
  viewportMode,
}: {
  expanded: boolean;
  pinId: string;
  url: string;
  viewportMode: LinkSidePanelViewportMode;
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

  const [nav, setNav] = React.useState<PinWebviewNavState>({
    canGoBack: false,
    canGoForward: false,
    currentUrl: url,
  });

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

  React.useEffect(() => {
    if (!expanded) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      toggleLinkSidePanelExpanded();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [expanded]);

  const currentUrl = nav.currentUrl || url;
  const expandLabel = expanded ? "Exit full screen" : "Expand link panel";

  async function handleInspect() {
    try {
      await inspectPinWebview(pinId);
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Could not open inspector.",
      );
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
        <div className="flex min-w-0 items-center gap-0.5">
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
          <div
            className="min-w-0 flex-1 truncate rounded-md border border-border bg-muted/40 px-2 py-0.5 font-mono text-2xs text-muted-foreground"
            data-testid="link-side-panel-url"
            title={currentUrl}
          >
            {currentUrl}
          </div>
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
          <ChromeIconButton
            aria-label={expandLabel}
            data-testid="link-side-panel-expand"
            onClick={() => toggleLinkSidePanelExpanded()}
            size="icon-xs"
            tooltip={expandLabel}
            type="button"
            variant="outline"
          >
            {expanded ? <Minimize2 /> : <Maximize2 />}
          </ChromeIconButton>
        </div>
        <div
          className="flex min-w-0 items-center gap-1"
          data-testid="link-side-panel-mode-row"
        >
          <ModeButton
            active={viewportMode === "desktop"}
            label="Desktop"
            onSelect={() => setLinkSidePanelViewportMode("desktop")}
            testId="link-side-panel-mode-desktop"
          />
          <ModeButton
            active={viewportMode === "responsive"}
            label="Responsive"
            onSelect={() => setLinkSidePanelViewportMode("responsive")}
            testId="link-side-panel-mode-responsive"
          />
          <ModeButton
            active={viewportMode === "mobile"}
            label="Mobile"
            onSelect={() => setLinkSidePanelViewportMode("mobile")}
            testId="link-side-panel-mode-mobile"
          />
        </div>
      </div>
    </TooltipProvider>
  );
}
