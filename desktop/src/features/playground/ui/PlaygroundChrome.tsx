import {
  ArrowLeft,
  ArrowRight,
  Camera,
  ChevronLeft,
  ChevronsDown,
  ChevronsUp,
  Copy,
  Inspect,
  AppWindow,
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
  playgroundAddressDisplay,
  playgroundAddressNavigation,
  splitLockedPlaygroundUrl,
  suffixFromCurrentUrl,
} from "../lib/addressBar";
import {
  playgroundChromeCollapseTooltip,
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
  playgroundScreenshotFile,
  stagePlaygroundScreenshotDraft,
} from "../lib/screenshot";
import {
  dismissPlayground,
  getLiveBrowserForSid,
  updatePlaygroundSessionNav,
} from "../lib/sessions";
import { syncAndAnnounceBrowserTabs } from "../lib/runtime";
import type { PlaygroundSession } from "../lib/sessions";
import { playgroundPin } from "../lib/types";
import type { PlaygroundNavState } from "../lib/types";
import {
  closePlaygroundWebviewInspect,
  getPlaygroundWebviewNavState,
  inspectPlaygroundWebview,
  PLAYGROUND_WEBVIEW_RESTORE_EVENT,
  playgroundWebviewGoBack,
  playgroundWebviewGoForward,
  playgroundWebviewNavigate,
  playgroundWebviewReload,
  screenshotPlaygroundWebview,
  subscribePlaygroundWebviewNav,
  currentWindowLabel,
} from "../lib/webview";
import {
  AGENT_DRIVING_CHROME_TOOLTIP,
  isAgentDrivingChromeLocked,
} from "@/features/browser-agent/lib/chromeLock";
import {
  getBrowserAgentGrant,
  subscribeBrowserAgentGrant,
} from "@/features/browser-agent/lib/api";
import { browserWebviewLabel } from "@/features/browser-agent/lib/labels";
import type { BrowserAgentGrant } from "@/features/browser-agent/lib/types";
import { BrowserAgentChrome } from "@/features/browser-agent/ui/BrowserAgentChrome";
import {
  PLAYGROUND_HULA,
  PLAYGROUND_VERSION,
} from "../lib/types";
import type { PlaygroundChromeMode } from "./PlaygroundStage";

export function PlaygroundChrome({
  conversation,
  docked,
  fullscreen,
  hideDismiss = false,
  hideDock = false,
  lockPlacement,
  mode,
  onModeChange,
  onStageResync,
  onToggleDock,
  onDetach,
  onToggleFullscreen,
  session,
  showDetach = false,
  showFullscreen = false,
  showInspect = false,
  tabs = null,
  agentGrantPrefill = null,
  agentDrivingNavLock,
  urlBarReadOnly = false,
}: {
  conversation: PlaygroundConversation | null;
  agentGrantPrefill?: { agentId: string; agentPubkey: string } | null;
  /** When set, forces chrome nav lock (layout tests). Live grant used when omitted. */
  agentDrivingNavLock?: boolean;
  /** Secondary tabs: full URL field locked (read-only). Main stays editable unless Drive locks. */
  urlBarReadOnly?: boolean;
  docked: boolean;
  fullscreen: boolean;
  hideDismiss?: boolean;
  hideDock?: boolean;
  lockPlacement?: "window" | "dock";
  mode: PlaygroundChromeMode;
  onDetach?: () => void;
  onModeChange: (mode: PlaygroundChromeMode) => void;
  onStageResync?: () => void;
  onToggleDock: () => void;
  onToggleFullscreen: () => void;
  session: PlaygroundSession;
  showDetach?: boolean;
  showFullscreen?: boolean;
  showInspect?: boolean;
  /** Tab strip rendered on the chrome tabs row (collapse sits to the right). */
  tabs?: React.ReactNode;
}) {
  const locked = splitLockedPlaygroundUrl(session.url);
  const [nav, setNav] = React.useState<PlaygroundNavState>({
    sid: session.sid,
    canGoBack: false,
    canGoForward: false,
    currentUrl: session.url,
  });
  const [suffix, setSuffix] = React.useState(locked.suffix);
  const canScreenshot = playgroundScreenshotAvailable(conversation);
  const pin = playgroundPin(session);
  const currentUrl = nav.currentUrl || session.url;
  const webviewLabel = browserWebviewLabel({
    surface: "playground",
    surfaceId: session.sid,
    windowLabel: currentWindowLabel(),
  });
  const [grant, setGrant] = React.useState<BrowserAgentGrant | null>(null);
  React.useEffect(() => {
    let cancelled = false;
    void getBrowserAgentGrant(webviewLabel).then((next) => {
      if (!cancelled) setGrant(next);
    });
    const unlisten = subscribeBrowserAgentGrant((payload) => {
      if (payload.webviewLabel !== webviewLabel) return;
      setGrant(payload.grant);
    });
    return () => {
      cancelled = true;
      void unlisten.then((stop) => stop());
    };
  }, [webviewLabel]);
  const agentDriving =
    agentDrivingNavLock !== undefined
      ? agentDrivingNavLock
      : isAgentDrivingChromeLocked(grant);
  const urlLocked = urlBarReadOnly || agentDriving;
  const [chromeCollapsed, setChromeCollapsed] = React.useState(false);
  function toggleChromeCollapsed() {
    setChromeCollapsed((value) => !value);
    onStageResync?.();
  }

  const urlLockTooltip = urlBarReadOnly
    ? "URL is locked on secondary tabs"
    : agentDriving
      ? AGENT_DRIVING_CHROME_TOOLTIP
      : undefined;
  const navTooltip = (id: "back" | "forward" | "refresh") =>
    agentDriving ? AGENT_DRIVING_CHROME_TOOLTIP : playgroundChromeTooltip(id);

  const wasFullscreenRef = React.useRef(fullscreen);
  React.useEffect(() => {
    const was = wasFullscreenRef.current;
    wasFullscreenRef.current = fullscreen;
    if (was && !fullscreen) {
      void closePlaygroundWebviewInspect(session.sid).catch(() => {});
    }
  }, [fullscreen, session.sid]);

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
      updatePlaygroundSessionNav(session.sid, {
        url: payload.currentUrl || undefined,
      });
      const browserId = getLiveBrowserForSid(session.sid)?.browserId;
      if (browserId) {
        void syncAndAnnounceBrowserTabs(browserId).catch(() => undefined);
      }
    });
    return () => {
      cancelled = true;
      void unlisten.then((stop) => stop());
    };
  }, [session.sid, session.url]);

  async function handleInspect() {
    if (!showInspect) return;
    try {
      await inspectPlaygroundWebview(session.sid);
      onStageResync?.();
      window.dispatchEvent(new Event(PLAYGROUND_WEBVIEW_RESTORE_EVENT));
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
    if (urlLocked) return;
    if (agentDriving) return;
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
      // Stage only — never park/hide the split/dock webview (JM: screenshot
      // was blanking the left pane via dismissPlayground).
      stagePlaygroundScreenshotDraft({ conversation, file });
      toast.success("Screenshot added to draft");
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : "Could not capture playground.",
      );
    }
  }

  const address = playgroundAddressDisplay(session.url, currentUrl);

  const collapseLabel = playgroundChromeCollapseTooltip(chromeCollapsed);

  return (
    <TooltipProvider>
      <header
        className={PLAYGROUND_CHROME_CLASS}
        data-chrome-collapsed={chromeCollapsed ? "true" : undefined}
        data-testid="playground-chrome"
        style={PLAYGROUND_OPAQUE_FILL_STYLE}
      >
        {/*
         * Tabs row: optional tab strip + collapse (URL / tooling) on the right.
         * Shared by overlay, channel idle-aux, and Browsers Open slide-out.
         */}
        <div
          className="-mx-2 -mt-1 flex min-h-7 min-w-0 shrink-0 items-center gap-0.5 border-b border-border/80 bg-muted/30 px-1"
          data-testid="playground-chrome-tabs-row"
        >
          {tabs}
          <div className="ml-auto flex shrink-0 items-center">
            <ChromeTooltipButton
              aria-expanded={!chromeCollapsed}
              aria-label={collapseLabel}
              data-testid="playground-chrome-collapse"
              onClick={toggleChromeCollapsed}
              size="icon-xs"
              tooltip={collapseLabel}
              type="button"
              variant="ghost"
            >
              {chromeCollapsed ? <ChevronsDown /> : <ChevronsUp />}
            </ChromeTooltipButton>
          </div>
        </div>
        {chromeCollapsed ? null : (
          <>
        {/*
         * Row 1: URL only (parity with link/pin slide-out). Locked host+path
         * + editable suffix while under the start lock; otherwise full URL
         * once — never locked-prefix + foreign-host double display.
         */}
        {address.mode === "locked" ? (
          <form
            className="flex min-w-0 items-stretch overflow-hidden rounded-md border border-border bg-muted/40"
            data-agent-driving={agentDriving ? "true" : undefined}
            data-url-locked={urlBarReadOnly ? "true" : undefined}
            data-testid="playground-address"
            onSubmit={handleAddressSubmit}
          >
            <span
              className="max-w-[55%] truncate bg-muted px-2 py-0.5 text-2xs text-muted-foreground"
              data-testid="playground-url-prefix"
            >
              {address.prefix}
            </span>
            <input
              aria-label="Playground path"
              className={
                urlLocked
                  ? "min-w-0 flex-1 cursor-not-allowed bg-transparent px-2 py-0.5 text-2xs text-foreground opacity-50 outline-none"
                  : "min-w-0 flex-1 bg-transparent px-2 py-0.5 text-2xs text-foreground outline-none"
              }
              data-testid="playground-url-suffix"
              disabled={urlLocked}
              onChange={(event) => setSuffix(event.target.value)}
              readOnly={urlLocked}
              title={urlLockTooltip}
              value={suffix}
            />
          </form>
        ) : (
          <div
            className="min-w-0 truncate rounded-md border border-border bg-muted/40 px-2 py-0.5 font-mono text-2xs text-muted-foreground"
            data-agent-driving={agentDriving ? "true" : undefined}
            data-url-locked={urlBarReadOnly ? "true" : undefined}
            data-testid="playground-address"
            title={urlLockTooltip ?? address.url}
          >
            <span data-testid="playground-url-full">{address.url}</span>
          </div>
        )}
        {/*
         * Row 2: Desktop/Responsive/Mobile left; tooling icons right-justified.
         * Agent Observe/Drive sits behind a Bot toggle in the tool strip; its
         * expanded controls render as a sibling row under this mode row.
         */}
        <BrowserAgentChrome
          channelId={conversation?.channelId ?? null}
          playgroundCard={{
            hula: PLAYGROUND_HULA,
            v: PLAYGROUND_VERSION,
            name: session.name,
            url: session.url,
            sid: session.sid,
            ...(session.pin ? { pin: session.pin } : {}),
            ...(session.stack ? { stack: session.stack } : {}),
            ...(session.expires != null ? { expires: session.expires } : {}),
          }}
          prefillAgent={agentGrantPrefill}
          surface="playground"
          surfaceId={session.sid}
          threadRoot={
            conversation?.draftKey?.startsWith("thread:")
              ? conversation.draftKey.slice("thread:".length)
              : null
          }
          variant="toolbar"
          windowLabel={currentWindowLabel()}
        >
          {(agentToggle) => (
            <div
              className={`relative flex min-w-0 shrink-0 items-center gap-1 py-0.5${
                lockPlacement != null ? " min-h-7" : ""
              }`}
              data-testid="playground-mode-row"
            >
              <div className="flex min-w-0 items-center gap-1">
                <ModeButton
                  active={mode === "desktop"}
                  disabled={agentDriving}
                  label="Desktop"
                  onSelect={() => onModeChange("desktop")}
                  testId="playground-mode-desktop"
                  tooltip={
                    agentDriving ? AGENT_DRIVING_CHROME_TOOLTIP : undefined
                  }
                />
                <ModeButton
                  active={mode === "responsive"}
                  disabled={agentDriving}
                  label="Responsive"
                  onSelect={() => onModeChange("responsive")}
                  testId="playground-mode-responsive"
                  tooltip={
                    agentDriving ? AGENT_DRIVING_CHROME_TOOLTIP : undefined
                  }
                />
                <ModeButton
                  active={mode === "mobile"}
                  disabled={agentDriving}
                  label="Mobile"
                  onSelect={() => onModeChange("mobile")}
                  testId="playground-mode-mobile"
                  tooltip={
                    agentDriving ? AGENT_DRIVING_CHROME_TOOLTIP : undefined
                  }
                />
              </div>
              {pin ? (
                <p
                  className="pointer-events-none absolute left-1/2 -translate-x-1/2 font-mono text-2xs text-muted-foreground"
                  data-testid="playground-chrome-pin"
                >
                  PIN {pin}
                </p>
              ) : null}
              <div
                className="ml-auto flex shrink-0 items-center gap-0.5"
                data-testid="playground-tool-icons"
              >
                <ChromeTooltipButton
                  aria-label={navTooltip("back")}
                  data-testid="playground-back"
                  disabled={agentDriving || !nav.canGoBack}
                  onClick={() => {
                    void playgroundWebviewGoBack(session.sid).then(setNav);
                  }}
                  size="icon-xs"
                  tooltip={navTooltip("back")}
                  type="button"
                  variant="ghost"
                >
                  <ArrowLeft />
                </ChromeTooltipButton>
                <ChromeTooltipButton
                  aria-label={navTooltip("forward")}
                  data-testid="playground-forward"
                  disabled={agentDriving || !nav.canGoForward}
                  onClick={() => {
                    void playgroundWebviewGoForward(session.sid).then(setNav);
                  }}
                  size="icon-xs"
                  tooltip={navTooltip("forward")}
                  type="button"
                  variant="ghost"
                >
                  <ArrowRight />
                </ChromeTooltipButton>
                <ChromeTooltipButton
                  aria-label={navTooltip("refresh")}
                  data-testid="playground-refresh"
                  disabled={agentDriving}
                  onClick={() => {
                    void playgroundWebviewReload(session.sid);
                  }}
                  size="icon-xs"
                  tooltip={navTooltip("refresh")}
                  type="button"
                  variant="ghost"
                >
                  <RefreshCw />
                </ChromeTooltipButton>
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
                {agentToggle}
                {showInspect ? (
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
                ) : null}
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
                {showDetach ? (
                  <ChromeTooltipButton
                    aria-label="Detach"
                    data-testid="playground-detach"
                    onClick={() => onDetach?.()}
                    size="icon-xs"
                    tooltip="Detach"
                    type="button"
                    variant="outline"
                  >
                    <AppWindow />
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
            </div>
          )}
        </BrowserAgentChrome>
          </>
        )}
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
  disabled = false,
  label,
  onSelect,
  testId,
  tooltip,
}: {
  active: boolean;
  disabled?: boolean;
  label: string;
  onSelect: () => void;
  testId: string;
  tooltip?: string;
}) {
  const button = (
    <button
      className={
        active
          ? "rounded-md bg-secondary px-2 py-0.5 text-xs disabled:cursor-not-allowed disabled:opacity-50"
          : "rounded-md px-2 py-0.5 text-xs text-muted-foreground hover:text-foreground disabled:cursor-not-allowed disabled:opacity-50 disabled:hover:text-muted-foreground"
      }
      data-testid={testId}
      disabled={disabled}
      onClick={() => {
        if (disabled) return;
        onSelect();
      }}
      title={disabled ? tooltip : undefined}
      type="button"
    >
      {label}
    </button>
  );
  if (!tooltip) return button;
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        {disabled ? <span className="inline-flex">{button}</span> : button}
      </TooltipTrigger>
      <TooltipContent side="bottom">{tooltip}</TooltipContent>
    </Tooltip>
  );
}
