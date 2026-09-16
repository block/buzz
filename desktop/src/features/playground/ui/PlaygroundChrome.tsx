import {
  ArrowLeft,
  ArrowRight,
  Camera,
  ChevronLeft,
  Copy,
  Inspect,
  Maximize2,
  Minimize2,
  PanelLeftClose,
  PanelLeftOpen,
  RefreshCw,
} from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { copyTextToClipboard } from "@/shared/lib/clipboard";
import { Button, type ButtonProps } from "@/shared/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/shared/ui/tooltip";

import {
  playgroundAddressNavigation,
  splitLockedPlaygroundUrl,
  suffixFromCurrentUrl,
} from "../lib/addressBar";
import {
  playgroundChromeTooltip,
  playgroundDockTooltip,
  playgroundFullscreenTooltip,
} from "../lib/chromeTooltips";
import type { PlaygroundConversation } from "../lib/conversation";
import { playgroundScreenshotAvailable } from "../lib/conversation";
import {
  PLAYGROUND_CHROME_CLASS,
  PLAYGROUND_OPAQUE_FILL_STYLE,
} from "../lib/overlayLayout";
import {
  dismissAndStagePlaygroundScreenshot,
  playgroundScreenshotFile,
} from "../lib/screenshot";
import { dismissPlayground, disposePlayground } from "../lib/sessions";
import type { PlaygroundSession } from "../lib/sessions";
import { playgroundPin } from "../lib/types";
import type { PlaygroundNavState } from "../lib/types";
import {
  getPlaygroundWebviewNavState,
  inspectPlaygroundWebview,
  playgroundWebviewGoBack,
  playgroundWebviewGoForward,
  playgroundWebviewNavigate,
  playgroundWebviewReload,
  screenshotPlaygroundWebview,
  subscribePlaygroundWebviewNav,
} from "../lib/webview";
import type { PlaygroundChromeMode } from "./PlaygroundStage";

export function PlaygroundChrome({
  conversation,
  docked,
  fullscreen,
  hideDismiss = false,
  hideDispose = false,
  hideDock = false,
  lockPlacement,
  mode,
  onModeChange,
  onStageResync,
  onToggleDock,
  onToggleFullscreen,
  session,
  showFullscreen = true,
}: {
  conversation: PlaygroundConversation | null;
  docked: boolean;
  fullscreen: boolean;
  hideDismiss?: boolean;
  hideDispose?: boolean;
  hideDock?: boolean;
  lockPlacement?: "window" | "dock";
  mode: PlaygroundChromeMode;
  onModeChange: (mode: PlaygroundChromeMode) => void;
  onStageResync?: () => void;
  onToggleDock: () => void;
  onToggleFullscreen: () => void;
  session: PlaygroundSession;
  showFullscreen?: boolean;
}) {
  const locked = splitLockedPlaygroundUrl(session.url);
  const [nav, setNav] = React.useState<PlaygroundNavState>({
    sid: session.sid,
    canGoBack: false,
    canGoForward: false,
    currentUrl: session.url,
  });
  const [suffix, setSuffix] = React.useState(locked.suffix);
  const [disposeArmed, setDisposeArmed] = React.useState(false);
  const canScreenshot = playgroundScreenshotAvailable(conversation);
  const pin = playgroundPin(session);
  const currentUrl = nav.currentUrl || session.url;

  React.useEffect(() => {
    let cancelled = false;
    void getPlaygroundWebviewNavState(session.sid, session.url).then(
      (state) => {
        if (cancelled) return;
        setNav(state);
        setSuffix(
          suffixFromCurrentUrl(session.url, state.currentUrl || session.url),
        );
      },
    );
    const unlisten = subscribePlaygroundWebviewNav((payload) => {
      if (payload.sid !== session.sid) return;
      setNav(payload);
      setSuffix(suffixFromCurrentUrl(session.url, payload.currentUrl));
    });
    return () => {
      cancelled = true;
      void unlisten.then((stop) => stop());
    };
  }, [session.sid, session.url]);

  async function handleInspect() {
    try {
      await inspectPlaygroundWebview(session.sid);
      onStageResync?.();
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : "Could not inspect playground.",
      );
    }
  }

  function handleAddressSubmit(event: React.FormEvent) {
    event.preventDefault();
    const next = playgroundAddressNavigation(session.url, suffix);
    if (!next.ok) {
      toast.error(next.message);
      return;
    }
    void playgroundWebviewNavigate(session.sid, next.url).then(setNav);
  }

  async function handleScreenshot() {
    if (!conversation) return;
    try {
      const result = await screenshotPlaygroundWebview(session.sid);
      const file = playgroundScreenshotFile(result);
      dismissAndStagePlaygroundScreenshot({ conversation, file });
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : "Could not capture playground.",
      );
    }
  }

  return (
    <TooltipProvider>
      <header
        className={PLAYGROUND_CHROME_CLASS}
        data-testid="playground-chrome"
        style={PLAYGROUND_OPAQUE_FILL_STYLE}
      >
        <div className="flex min-w-0 items-center gap-1">
          {hideDispose ? null : disposeArmed ? (
            <div className="flex shrink-0 items-center gap-0.5">
              <Button
                onClick={() => setDisposeArmed(false)}
                size="xs"
                type="button"
                variant="ghost"
              >
                Cancel
              </Button>
              <Button
                data-testid="playground-dispose-confirm"
                onClick={() => disposePlayground(session.sid)}
                size="xs"
                type="button"
                variant="destructive"
              >
                Confirm dispose
              </Button>
            </div>
          ) : (
            <ChromeTooltipButton
              data-testid="playground-dispose"
              onClick={() => setDisposeArmed(true)}
              size="xs"
              tooltip={playgroundChromeTooltip("dispose")}
              type="button"
              variant="destructive"
            >
              Dispose
            </ChromeTooltipButton>
          )}
          <div className="flex shrink-0 items-center gap-0.5">
            <ChromeTooltipButton
              aria-label={playgroundChromeTooltip("back")}
              data-testid="playground-back"
              disabled={!nav.canGoBack}
              onClick={() => {
                void playgroundWebviewGoBack(session.sid).then(setNav);
              }}
              size="icon-xs"
              tooltip={playgroundChromeTooltip("back")}
              type="button"
              variant="ghost"
            >
              <ArrowLeft />
            </ChromeTooltipButton>
            <ChromeTooltipButton
              aria-label={playgroundChromeTooltip("forward")}
              data-testid="playground-forward"
              disabled={!nav.canGoForward}
              onClick={() => {
                void playgroundWebviewGoForward(session.sid).then(setNav);
              }}
              size="icon-xs"
              tooltip={playgroundChromeTooltip("forward")}
              type="button"
              variant="ghost"
            >
              <ArrowRight />
            </ChromeTooltipButton>
            <ChromeTooltipButton
              aria-label={playgroundChromeTooltip("refresh")}
              data-testid="playground-refresh"
              onClick={() => {
                void playgroundWebviewReload(session.sid);
              }}
              size="icon-xs"
              tooltip={playgroundChromeTooltip("refresh")}
              type="button"
              variant="ghost"
            >
              <RefreshCw />
            </ChromeTooltipButton>
          </div>
          <form
            className="flex min-w-0 flex-1 items-stretch overflow-hidden rounded-md border border-border bg-muted/40"
            data-testid="playground-address"
            onSubmit={handleAddressSubmit}
          >
            <span
              className="max-w-[55%] truncate bg-muted px-2 py-0.5 text-2xs text-muted-foreground"
              data-testid="playground-url-prefix"
            >
              {locked.prefix}
            </span>
            <input
              aria-label="Playground path"
              className="min-w-0 flex-1 bg-transparent px-2 py-0.5 text-2xs text-foreground outline-none"
              data-testid="playground-url-suffix"
              onChange={(event) => setSuffix(event.target.value)}
              value={suffix}
            />
          </form>
          <ChromeTooltipButton
            aria-label={playgroundChromeTooltip("copy")}
            data-testid="playground-copy-url"
            onClick={() => copyTextToClipboard(currentUrl, "URL copied")}
            size="icon-xs"
            tooltip={playgroundChromeTooltip("copy")}
            type="button"
            variant="ghost"
          >
            <Copy />
          </ChromeTooltipButton>
          <ChromeTooltipButton
            aria-label={playgroundChromeTooltip("inspect")}
            data-testid="playground-inspect"
            onClick={() => void handleInspect()}
            size="icon-xs"
            tooltip={playgroundChromeTooltip("inspect")}
            type="button"
            variant="outline"
          >
            <Inspect />
          </ChromeTooltipButton>
          {canScreenshot ? (
            <ChromeTooltipButton
              aria-label={playgroundChromeTooltip("screenshot")}
              data-testid="playground-screenshot"
              onClick={handleScreenshot}
              size="icon-xs"
              tooltip={playgroundChromeTooltip("screenshot")}
              type="button"
              variant="outline"
            >
              <Camera />
            </ChromeTooltipButton>
          ) : null}
          {showFullscreen ? (
            <ChromeTooltipButton
              aria-label={playgroundFullscreenTooltip(fullscreen)}
              data-testid="playground-fullscreen"
              onClick={onToggleFullscreen}
              size="icon-xs"
              tooltip={playgroundFullscreenTooltip(fullscreen)}
              type="button"
              variant="outline"
            >
              {fullscreen ? <Minimize2 /> : <Maximize2 />}
            </ChromeTooltipButton>
          ) : null}
          {hideDock || fullscreen ? null : (
            <ChromeTooltipButton
              aria-label={playgroundDockTooltip(docked)}
              data-testid="playground-dock"
              onClick={onToggleDock}
              size="icon-xs"
              tooltip={playgroundDockTooltip(docked)}
              type="button"
              variant="outline"
            >
              {docked ? <PanelLeftOpen /> : <PanelLeftClose />}
            </ChromeTooltipButton>
          )}
          {hideDismiss ? null : (
            <ChromeTooltipButton
              aria-label={playgroundChromeTooltip("dismiss")}
              data-testid="playground-dismiss"
              onClick={() => dismissPlayground()}
              size="icon-xs"
              tooltip={playgroundChromeTooltip("dismiss")}
              type="button"
              variant="outline"
            >
              <ChevronLeft />
            </ChromeTooltipButton>
          )}
        </div>
        <div
          className={`relative flex min-w-0 shrink-0 items-center justify-start gap-1 py-0.5${
            lockPlacement != null ? " min-h-7" : ""
          }`}
          data-testid="playground-mode-row"
        >
          <ModeButton
            active={mode === "desktop"}
            label="Desktop"
            onSelect={() => onModeChange("desktop")}
            testId="playground-mode-desktop"
          />
          <ModeButton
            active={mode === "responsive"}
            label="Responsive"
            onSelect={() => onModeChange("responsive")}
            testId="playground-mode-responsive"
          />
          <ModeButton
            active={mode === "mobile"}
            label="Mobile"
            onSelect={() => onModeChange("mobile")}
            testId="playground-mode-mobile"
          />
          {pin ? (
            <p
              className="pointer-events-none absolute left-1/2 -translate-x-1/2 font-mono text-2xs text-muted-foreground"
              data-testid="playground-chrome-pin"
            >
              PIN {pin}
            </p>
          ) : null}
        </div>
      </header>
    </TooltipProvider>
  );
}

function ChromeTooltipButton({
  children,
  tooltip,
  ...props
}: ButtonProps & { tooltip: string }) {
  const button = <Button {...props}>{children}</Button>;
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        {props.disabled ? (
          <span className="inline-flex">{button}</span>
        ) : (
          button
        )}
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
