import {
  ArrowLeft,
  ArrowRight,
  Copy,
  Eye,
  EyeOff,
  ExternalLink,
  Home,
  Maximize2,
  MoreHorizontal,
  Minimize2,
  Pin,
  RotateCw,
  X,
} from "lucide-react";

import { useBusinessDock } from "@/features/business-dock/BusinessDockProvider";
import { useWorkbenchAuth } from "@/features/workbench-auth/WorkbenchAuthProvider";
import {
  buildBusinessReference,
  buildBusinessUrl,
  formatBusinessResourceLabel,
} from "@/features/business-dock/businessResourceResolver";
import { cn } from "@/shared/lib/cn";
import { copyTextToClipboard } from "@/shared/lib/clipboard";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";

const TOOLBAR_BUTTON_CLASS = "h-8 w-8 shrink-0";

export function BusinessDockToolbar() {
  const {
    canGoBack,
    canGoForward,
    close,
    config,
    goBack,
    goForward,
    goHome,
    logoutBusiness,
    openCurrentInBrowser,
    refresh,
    state,
    toggleFollowConversation,
    toggleFullscreen,
    togglePinned,
  } = useBusinessDock();
  const {
    phase: workbenchAuthPhase,
    signOut,
    signOutWorkbench,
  } = useWorkbenchAuth();

  const resourceLabel = state.currentResource
    ? formatBusinessResourceLabel(state.currentResource)
    : state.title || "Business system";
  const copyReference = () => {
    if (!config) return;
    const reference = state.currentResource
      ? (buildBusinessReference(state.currentResource) ??
        buildBusinessUrl(state.currentResource, config))
      : state.currentUrl;
    if (reference) copyTextToClipboard(reference, "Business reference copied");
  };

  return (
    <div
      className="flex h-11 shrink-0 cursor-default select-none items-center gap-0.5 border-b border-border/50 bg-background/90 px-2 backdrop-blur-md"
      data-testid="business-dock-toolbar"
    >
      <Button
        aria-label="Business home"
        className={TOOLBAR_BUTTON_CLASS}
        disabled={!config}
        onClick={goHome}
        size="icon"
        title="Business home"
        variant="ghost"
      >
        <Home />
      </Button>
      <Button
        aria-label="Business back"
        className={TOOLBAR_BUTTON_CLASS}
        disabled={!canGoBack}
        onClick={goBack}
        size="icon"
        title="Back"
        variant="ghost"
      >
        <ArrowLeft />
      </Button>
      <Button
        aria-label="Business forward"
        className={TOOLBAR_BUTTON_CLASS}
        disabled={!canGoForward}
        onClick={goForward}
        size="icon"
        title="Forward"
        variant="ghost"
      >
        <ArrowRight />
      </Button>
      <Button
        aria-label="Refresh business system"
        className={TOOLBAR_BUTTON_CLASS}
        disabled={!config}
        onClick={refresh}
        size="icon"
        title="Refresh"
        variant="ghost"
      >
        <RotateCw />
      </Button>
      <div className="min-w-0 flex-1 px-2 text-center">
        <p
          className="flex items-center justify-center gap-1.5 truncate text-sm font-medium"
          title={resourceLabel}
        >
          {state.dirty ? (
            <>
              <span
                aria-hidden="true"
                className="size-1.5 shrink-0 rounded-full bg-amber-500"
                data-testid="business-dock-dirty-indicator"
              />
              <span className="sr-only">Unsaved business changes</span>
            </>
          ) : null}
          <span className="truncate" data-testid="business-resource-label">
            {resourceLabel}
          </span>
        </p>
      </div>
      <Button
        aria-label={
          state.followConversation
            ? "Stop following conversation business links"
            : "Follow conversation business links"
        }
        aria-pressed={state.followConversation}
        className={cn(
          TOOLBAR_BUTTON_CLASS,
          state.followConversation && "text-primary",
        )}
        onClick={toggleFollowConversation}
        size="icon"
        title={
          state.followConversation
            ? "Following conversation"
            : "Follow conversation"
        }
        variant="ghost"
      >
        {state.followConversation ? <Eye /> : <EyeOff />}
      </Button>
      <Button
        aria-label={state.pinned ? "Unpin Business Dock" : "Pin Business Dock"}
        aria-pressed={state.pinned}
        className={cn(TOOLBAR_BUTTON_CLASS, state.pinned && "text-primary")}
        onClick={togglePinned}
        size="icon"
        title={state.pinned ? "Unpin" : "Pin"}
        variant="ghost"
      >
        <Pin />
      </Button>
      <Button
        aria-label={
          state.fullscreen
            ? "Exit full screen business system"
            : "Full screen business system"
        }
        className={TOOLBAR_BUTTON_CLASS}
        onClick={toggleFullscreen}
        size="icon"
        title={state.fullscreen ? "Exit full screen" : "Full screen"}
        variant="ghost"
      >
        {state.fullscreen ? <Minimize2 /> : <Maximize2 />}
      </Button>
      <Button
        aria-label="Copy Business Reference"
        className={TOOLBAR_BUTTON_CLASS}
        disabled={!config || (!state.currentResource && !state.currentUrl)}
        onClick={copyReference}
        size="icon"
        title="Copy Business Reference"
        variant="ghost"
      >
        <Copy />
      </Button>
      <Button
        aria-label="Open business system in browser"
        className={TOOLBAR_BUTTON_CLASS}
        disabled={!config}
        onClick={openCurrentInBrowser}
        size="icon"
        title="Open in browser"
        variant="ghost"
      >
        <ExternalLink />
      </Button>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            aria-label="Business session menu"
            className={TOOLBAR_BUTTON_CLASS}
            size="icon"
            title="Session options"
            variant="ghost"
          >
            <MoreHorizontal />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-64">
          <DropdownMenuItem
            disabled={workbenchAuthPhase !== "authenticated"}
            onSelect={logoutBusiness}
          >
            Sign out of Business
          </DropdownMenuItem>
          <DropdownMenuItem
            disabled={workbenchAuthPhase !== "authenticated"}
            onSelect={() => void signOutWorkbench()}
          >
            Sign out of Workbench (keep SSO)
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem
            disabled={workbenchAuthPhase !== "authenticated"}
            onSelect={() => void signOut()}
          >
            Sign out of all enterprise apps
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <Button
        aria-label="Close Business Dock"
        className={TOOLBAR_BUTTON_CLASS}
        onClick={close}
        size="icon"
        title="Close"
        variant="ghost"
      >
        <X />
      </Button>
    </div>
  );
}
