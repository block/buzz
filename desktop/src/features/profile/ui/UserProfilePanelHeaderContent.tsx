import type { ReactNode } from "react";

import { CopyButton } from "@/features/agents/ui/CopyButton";
import { MemoryRefreshButton } from "@/features/agent-memory/ui/MemorySection";
import {
  PROFILE_PANEL_VIEW_TITLES,
  type ProfilePanelView,
} from "@/features/profile/ui/UserProfilePanelUtils";
import {
  AuxiliaryPanelHeaderActions,
  AuxiliaryPanelHeaderCloseButton,
  AuxiliaryPanelHeaderGroup,
  AuxiliaryPanelHeaderTitleBlock,
} from "@/shared/layout/AuxiliaryPanelHeader";

export function getUserProfilePanelHeaderContent({
  agentSettingsMenu,
  effectivePubkey,
  logCopyValue,
  logSubtitle,
  onBack,
  onClose,
  view,
  viewerIsOwner,
}: {
  agentSettingsMenu: ReactNode;
  effectivePubkey: string | null;
  logCopyValue?: string | null;
  logSubtitle?: string | null;
  onBack: () => void;
  onClose: () => void;
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
      <AuxiliaryPanelHeaderCloseButton
        ariaLabel="Close profile"
        onClose={onClose}
        testId="user-profile-panel-close"
      />
    </AuxiliaryPanelHeaderActions>
  );

  return { headerActions, headerLeftContent };
}
