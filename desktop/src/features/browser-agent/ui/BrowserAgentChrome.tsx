import { Bot, Eye, Hand, MousePointer2 } from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { useManagedAgentsQuery } from "@/features/agents/hooks";
import { Button } from "@/shared/ui/button";
import { cn } from "@/shared/lib/cn";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/shared/ui/tooltip";

import { invoke, isTauri } from "@tauri-apps/api/core";
import {
  clearBrowserAgentGrant,
  getBrowserAgentGrant,
  releaseBrowserAgentControl,
  setBrowserAgentGrant,
  subscribeBrowserAgentGrant,
  takeBrowserAgentControl,
} from "../lib/api";
import { browserWebviewLabel } from "../lib/labels";
import type { PlaygroundCard } from "@/features/playground/lib/types";

import { useRunbookGrantBridge } from "@/features/site-runbook/lib/useRunbookGrantBridge";

import { useDriveActivity } from "../lib/useDriveActivity";
import type { BrowserAgentGrant, BrowserAgentMode, BrowserAgentSurface } from "../lib/types";
import {
  BrowserAgentGrantDialog,
  type BrowserAgentGrantPick,
} from "./BrowserAgentGrantDialog";

const AGENT_TOGGLE_TOOLTIP = "Observe & Drive";

export function BrowserAgentChrome({
  channelId,
  children,
  playgroundCard = null,
  prefillAgent,
  showTakeRelease = true,
  surface,
  surfaceId,
  threadRoot,
  variant = "inline",
  windowLabel,
}: {
  channelId?: string | null;
  /**
   * Toolbar only: place the Agent toggle in the parent chrome (e.g. next to
   * Detach / Inspect). The expanded controls render as a sibling after that
   * row inside the same flex-col header.
   */
  children?: (toggle: React.ReactNode) => React.ReactNode;
  /** Enables pin-to-conversation in the grant dialog (playground only). */
  playgroundCard?: PlaygroundCard | null;
  prefillAgent?: { agentId: string; agentPubkey: string } | null;
  /**
   * When false, hide Take/Release control (Browsers list Bot panel). Keep
   * true in live playground chrome where the user is looking at the page.
   */
  showTakeRelease?: boolean;
  surface: BrowserAgentSurface;
  surfaceId: string;
  threadRoot?: string | null;
  /** `toolbar` = icon toggle + expandable second row; `inline` = compact chips. */
  variant?: "inline" | "toolbar";
  /** Target OS/main window for the webview label (detached playgrounds). */
  windowLabel?: string;
}) {
  const webviewLabel = browserWebviewLabel({
    surface,
    surfaceId,
    windowLabel,
  });
  const [grant, setGrant] = React.useState<BrowserAgentGrant | null>(null);
  const [dialogMode, setDialogMode] = React.useState<BrowserAgentMode | null>(
    null,
  );
  const [busy, setBusy] = React.useState(false);
  const [panelOpen, setPanelOpen] = React.useState(false);
  const agentsQuery = useManagedAgentsQuery();
  const expandAfterGrantRef = React.useRef(false);

  const agentDisplayName = React.useMemo(() => {
    if (!grant) return null;
    const agents = agentsQuery.data ?? [];
    const match =
      agents.find((agent) => agent.pubkey === grant.agentPubkey) ??
      agents.find((agent) => agent.pubkey === grant.agentId);
    const name = match?.name?.trim();
    if (name) return name;
    return grant.agentPubkey.slice(0, 8);
  }, [agentsQuery.data, grant]);

  useRunbookGrantBridge(grant, webviewLabel);

  const modeForActivity: BrowserAgentMode | "off" = grant?.mode ?? "off";
  const activityText = useDriveActivity(
    webviewLabel,
    modeForActivity,
    showTakeRelease,
  );
  // Keep last caption briefly so opacity can fade on clear (Finished → empty).
  const [activityShown, setActivityShown] = React.useState<string | null>(null);
  const [activityOpaque, setActivityOpaque] = React.useState(false);
  React.useEffect(() => {
    if (activityText) {
      setActivityShown(activityText);
      setActivityOpaque(true);
      return;
    }
    setActivityOpaque(false);
    const t = window.setTimeout(() => setActivityShown(null), 280);
    return () => window.clearTimeout(t);
  }, [activityText]);

  React.useEffect(() => {
    // Drive inbox + snapshot-request drain. Skip while human has Taken control
    // (Drive lock paused); Observe still drains snapshot requests.
    if (!grant) return;
    if (grant.mode === "drive" && grant.userHasControl) return;
    if (!isTauri() && import.meta.env.MODE !== "e2e") return;
    const timer = window.setInterval(() => {
      void invoke("browser_agent_process_drive_inbox", {
        webviewLabel,
      }).catch(() => {});
    }, 750);
    return () => window.clearInterval(timer);
  }, [grant, webviewLabel]);

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

  React.useEffect(() => {
    if (!expandAfterGrantRef.current || !grant) return;
    expandAfterGrantRef.current = false;
    if (variant === "toolbar") setPanelOpen(true);
  }, [grant, variant]);

  async function applyMode(mode: BrowserAgentMode | "off") {
    if (busy) return;
    if (mode === "off") {
      setBusy(true);
      try {
        await clearBrowserAgentGrant(webviewLabel);
        setGrant(null);
      } catch (error) {
        toast.error(error instanceof Error ? error.message : "Could not clear grant.");
      } finally {
        setBusy(false);
      }
      return;
    }
    if (prefillAgent && channelId) {
      const agents = agentsQuery.data ?? [];
      const match =
        agents.find((agent) => agent.pubkey === prefillAgent.agentPubkey) ??
        agents.find((agent) => agent.pubkey === prefillAgent.agentId);
      expandAfterGrantRef.current = true;
      await commitGrant({
        agentId: prefillAgent.agentId,
        agentPubkey: prefillAgent.agentPubkey,
        agentName:
          match?.name?.trim() || prefillAgent.agentPubkey.slice(0, 12),
        channelId,
        threadRoot: threadRoot ?? null,
        mode,
      });
      return;
    }
    setDialogMode(mode);
  }

  async function commitGrant(pick: BrowserAgentGrantPick, allowReplace = false) {
    setBusy(true);
    expandAfterGrantRef.current = true;
    try {
      const next = await setBrowserAgentGrant({
        surface,
        surfaceId,
        agentId: pick.agentId,
        agentPubkey: pick.agentPubkey,
        channelId: pick.channelId,
        threadRoot: pick.threadRoot,
        mode: pick.mode,
        allowReplace,
        ...(windowLabel ? { windowLabel } : {}),
      });
      setGrant(next);
      const base =
        pick.mode === "drive"
          ? `Drive granted to ${pick.agentName}`
          : `Observe granted to ${pick.agentName}`;
      toast.success(
        pick.pinnedToConversation ? `${base} · pinned to conversation` : base,
      );
    } catch (error) {
      const message =
        error instanceof Error ? error.message : "Could not set grant.";
      if (message.includes("confirm replace")) {
        if (window.confirm("Replace the current agent grant on this browser?")) {
          await commitGrant(pick, true);
        }
      } else {
        toast.error(message);
      }
    } finally {
      setBusy(false);
    }
  }

  async function handleTakeControl() {
    setBusy(true);
    try {
      const next = await takeBrowserAgentControl(webviewLabel);
      setGrant(next);
      toast.message("You have control — grant stays Drive");
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Could not take control.",
      );
    } finally {
      setBusy(false);
    }
  }

  async function handleReleaseControl() {
    setBusy(true);
    try {
      const next = await releaseBrowserAgentControl(webviewLabel);
      setGrant(next);
      toast.message("Agent is driving again");
    } catch (error) {
      toast.error(
        error instanceof Error ? error.message : "Could not release control.",
      );
    } finally {
      setBusy(false);
    }
  }

  const mode: BrowserAgentMode | "off" = grant?.mode ?? "off";
  const userHasControl = Boolean(grant?.mode === "drive" && grant.userHasControl);
  const hasGrant = grant != null;

  const controls = (
    <>
      <ModeChip
        active={mode === "off"}
        disabled={busy}
        label="Off"
        onClick={() => void applyMode("off")}
        testId="browser-agent-mode-off"
      />
      <ModeChip
        active={mode === "observe"}
        disabled={busy}
        icon={<Eye className="h-3 w-3" />}
        label="Observe"
        onClick={() => void applyMode("observe")}
        testId="browser-agent-mode-observe"
      />
      <ModeChip
        active={mode === "drive"}
        disabled={busy}
        icon={<MousePointer2 className="h-3 w-3" />}
        label="Drive"
        onClick={() => void applyMode("drive")}
        testId="browser-agent-mode-drive"
      />
      {grant && agentDisplayName ? (
        <span
          className={cn(
            "ml-1 truncate rounded-full px-2 py-0.5 text-2xs",
            grant.mode === "drive"
              ? "bg-orange-500/15 font-medium text-orange-700 dark:text-orange-400"
              : "bg-muted text-muted-foreground",
          )}
          data-testid="browser-agent-status-chip"
          title={
            grant.mode === "drive"
              ? userHasControl
                ? `Drive · ${agentDisplayName} · you have control`
                : `Drive · ${agentDisplayName}`
              : `Observe · ${agentDisplayName}`
          }
        >
          {grant.mode === "drive"
            ? userHasControl
              ? `Drive · ${agentDisplayName} · you`
              : `Drive · ${agentDisplayName}`
            : `Observe · ${agentDisplayName}`}
        </span>
      ) : null}
      {showTakeRelease && grant?.mode === "drive" ? (
        userHasControl ? (
          <Button
            data-testid="browser-agent-release-control"
            disabled={busy}
            onClick={() => void handleReleaseControl()}
            size="xs"
            type="button"
            variant="secondary"
          >
            <Hand className="mr-1 h-3 w-3" />
            Release control
          </Button>
        ) : (
          <Button
            data-testid="browser-agent-take-control"
            disabled={busy}
            onClick={() => void handleTakeControl()}
            size="xs"
            type="button"
            variant="secondary"
          >
            <Hand className="mr-1 h-3 w-3" />
            Take control
          </Button>
        )
      ) : null}
      {showTakeRelease && activityShown ? (
        <span
          aria-live="polite"
          className={cn(
            "ml-1 min-w-0 max-w-[14rem] truncate text-2xs text-muted-foreground transition-opacity duration-300",
            activityOpaque ? "opacity-100" : "opacity-0",
          )}
          data-testid="browser-agent-activity"
          title={activityShown}
        >
          {activityShown}
        </span>
      ) : null}
    </>
  );

  const grantDialog =
    dialogMode != null ? (
      <BrowserAgentGrantDialog
        channelId={channelId ?? ""}
        mode={dialogMode}
        onOpenChange={(next) => {
          if (!next) setDialogMode(null);
        }}
        onPick={(pick) => {
          void commitGrant(pick);
        }}
        open
        playgroundCard={surface === "playground" ? playgroundCard : null}
        threadRoot={threadRoot}
      />
    ) : null;

  if (variant === "inline") {
    return (
      <div
        className="flex min-w-0 items-center gap-1"
        data-testid="browser-agent-chrome"
      >
        {controls}
        {grantDialog}
      </div>
    );
  }

  const toggleButton = (
    <Button
      aria-expanded={panelOpen}
      aria-label={AGENT_TOGGLE_TOOLTIP}
      className={
        hasGrant
          ? "border-orange-500/50 text-orange-700 hover:bg-orange-500/10 dark:text-orange-400"
          : undefined
      }
      data-testid="browser-agent-toggle"
      onClick={() => setPanelOpen((open) => !open)}
      size="icon-xs"
      type="button"
      variant="outline"
    >
      <Bot />
    </Button>
  );

  const toggle = (
    <Tooltip>
      <TooltipTrigger asChild>{toggleButton}</TooltipTrigger>
      <TooltipContent side="bottom">{AGENT_TOGGLE_TOOLTIP}</TooltipContent>
    </Tooltip>
  );

  return (
    <>
      {children ? children(toggle) : toggle}
      {panelOpen ? (
        <div
          className="flex min-w-0 flex-wrap items-center gap-1 py-0.5"
          data-testid="browser-agent-chrome"
        >
          {controls}
        </div>
      ) : null}
      {grantDialog}
    </>
  );
}

function ModeChip({
  active,
  disabled,
  icon,
  label,
  onClick,
  testId,
}: {
  active: boolean;
  disabled?: boolean;
  icon?: React.ReactNode;
  label: string;
  onClick: () => void;
  testId: string;
}) {
  return (
    <Button
      aria-pressed={active}
      className={active ? "bg-muted" : undefined}
      data-testid={testId}
      disabled={disabled}
      onClick={onClick}
      size="xs"
      type="button"
      variant={active ? "secondary" : "ghost"}
    >
      {icon}
      <span className={icon ? "ml-1" : undefined}>{label}</span>
    </Button>
  );
}
