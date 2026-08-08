import { Eraser } from "lucide-react";
import type { ReactNode } from "react";

import { CopyButton } from "@/features/agents/ui/CopyButton";
import { Button } from "@/shared/ui/button";
import { MemoryRefreshButton } from "@/features/agent-memory/ui/MemorySection";
import {
  PROFILE_PANEL_VIEW_TITLES,
  type ProfilePanelView,
} from "@/features/profile/ui/UserProfilePanelUtils";
import {
  AuxiliaryPanelHeaderActions,
  AuxiliaryPanelHeaderGroup,
  AuxiliaryPanelHeaderTitleBlock,
} from "@/shared/layout/AuxiliaryPanel";

export function getUserProfilePanelHeaderContent({
  agentSettingsMenu,
  effectivePubkey,
  isLogClearPending,
  logCopyValue,
  logSubtitle,
  onBack,
  onClearLog,
  view,
  viewerIsOwner,
}: {
  agentSettingsMenu: ReactNode;
  effectivePubkey: string | null;
  isLogClearPending?: boolean;
  logCopyValue?: string | null;
  logSubtitle?: string | null;
  onBack: () => void;
  onClearLog?: () => void;
  view: ProfilePanelView;
  viewerIsOwner: boolean;
}) {
  const title = PROFILE_PANEL_VIEW_TITLES[view];
  const shouldShowLogDetails =
    (view === "diagnostics" || view === "logs") && Boolean(logSubtitle);
  const headerLeftContent = (
    <AuxiliaryPanelHeaderGroup
      align={shouldShowLogDetails ? "start" : "center"}
      backButtonAriaLabel="Back to profile"
      backButtonTestId="user-profile-panel-back"
      onBack={view !== "summary" ? onBack : undefined}
    >
      <AuxiliaryPanelHeaderTitleBlock
        subtitle={shouldShowLogDetails ? logSubtitle : null}
        subtitleTitle={logSubtitle ?? undefined}
        title={title}
      />
    </AuxiliaryPanelHeaderGroup>
  );
  const headerActions = (
    <AuxiliaryPanelHeaderActions>
      {view === "memories" && viewerIsOwner && effectivePubkey ? (
        <MemoryRefreshButton
          agentPubkey={effectivePubkey}
          variant="outline"
          viewerIsOwner={viewerIsOwner}
        />
      ) : null}
      {view === "summary" ? agentSettingsMenu : null}
      {shouldShowLogDetails && onClearLog ? (
        <Button
          className="text-muted-foreground hover:text-foreground"
          data-testid="user-profile-clear-log"
          disabled={isLogClearPending}
          onClick={onClearLog}
          size="icon"
          type="button"
          variant="ghost"
        >
          <Eraser className="h-4 w-4" />
          <span className="sr-only">Clear log</span>
        </Button>
      ) : null}
      {shouldShowLogDetails ? (
        <CopyButton
          className="text-muted-foreground hover:text-foreground"
          iconOnly
          label="Copy log"
          size="icon"
          value={logCopyValue ?? ""}
          variant="ghost"
        />
      ) : null}
    </AuxiliaryPanelHeaderActions>
  );

  return { headerActions, headerLeftContent };
}
