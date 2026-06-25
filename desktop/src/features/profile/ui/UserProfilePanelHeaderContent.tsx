import type { ReactNode } from "react";
import { ArrowLeft, X } from "lucide-react";

import { MemoryRefreshButton } from "@/features/agent-memory/ui/MemorySection";
import {
  PROFILE_PANEL_VIEW_TITLES,
  type ProfilePanelView,
} from "@/features/profile/ui/UserProfilePanelUtils";
import {
  AuxiliaryPanelHeaderGroup,
  AuxiliaryPanelTitle,
} from "@/shared/layout/AuxiliaryPanelHeader";
import { Button } from "@/shared/ui/button";

export function getUserProfilePanelHeaderContent({
  agentSettingsMenu,
  effectivePubkey,
  onBack,
  onClose,
  view,
  viewerIsOwner,
}: {
  agentSettingsMenu: ReactNode;
  effectivePubkey: string | null;
  onBack: () => void;
  onClose: () => void;
  view: ProfilePanelView;
  viewerIsOwner: boolean;
}) {
  const headerLeftContent = (
    <AuxiliaryPanelHeaderGroup>
      {view !== "summary" ? (
        <Button
          aria-label="Back to profile"
          className="shrink-0"
          data-testid="user-profile-panel-back"
          onClick={onBack}
          size="icon"
          type="button"
          variant="outline"
        >
          <ArrowLeft />
        </Button>
      ) : null}
      <AuxiliaryPanelTitle>
        {PROFILE_PANEL_VIEW_TITLES[view]}
      </AuxiliaryPanelTitle>
    </AuxiliaryPanelHeaderGroup>
  );
  const headerActions = (
    <div className="ml-auto flex shrink-0 items-center gap-2">
      {view === "memories" && viewerIsOwner && effectivePubkey ? (
        <MemoryRefreshButton
          agentPubkey={effectivePubkey}
          variant="outline"
          viewerIsOwner={viewerIsOwner}
        />
      ) : null}
      {view === "summary" ? agentSettingsMenu : null}
      <Button
        aria-label="Close profile"
        data-testid="user-profile-panel-close"
        onClick={onClose}
        size="icon"
        type="button"
        variant="ghost"
      >
        <X />
      </Button>
    </div>
  );

  return { headerActions, headerLeftContent };
}
