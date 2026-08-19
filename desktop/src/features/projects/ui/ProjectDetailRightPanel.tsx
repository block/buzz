import type * as React from "react";

import { normalizeRelayUrl } from "@/features/communities/communityStorage";
import { useCommunities } from "@/features/communities/useCommunities";
import { useIdentityQuery } from "@/shared/api/hooks";
import { normalizePubkey } from "@/shared/lib/pubkey";
import type { ProjectDetailAgentContext } from "@/features/projects/lib/projectDetailAgentContext";
import { ProjectAgentChatPanel } from "./ProjectAgentChatPanel";
import { ProjectRepositoryActionsPanel } from "./ProjectRepositoryActionsPanel";
import type { ProjectRightPanelMode } from "./ProjectRightPanelControls";

type RepositoryPanelProps = React.ComponentProps<
  typeof ProjectRepositoryActionsPanel
>;

export function ProjectDetailRightPanel({
  context,
  detachedRepository = false,
  mode,
  sharedHeaderBackdrop,
  ...repositoryProps
}: RepositoryPanelProps & {
  context: ProjectDetailAgentContext;
  detachedRepository?: boolean;
  mode: ProjectRightPanelMode;
  sharedHeaderBackdrop?: boolean;
}) {
  const { activeCommunity } = useCommunities();
  const identityQuery = useIdentityQuery();
  if (mode === "chat") {
    // Remount on community identity as well as repository address: the same
    // repo coordinate can exist in two communities, and retained panel state
    // (conversation, opener) must never cross that tenant boundary.
    const relayScope = activeCommunity?.relayUrl
      ? normalizeRelayUrl(activeCommunity.relayUrl)
      : "";
    const signerScope = identityQuery.data?.pubkey
      ? normalizePubkey(identityQuery.data.pubkey)
      : "";
    return (
      <ProjectAgentChatPanel
        canResetWidth={repositoryProps.canResetWidth}
        context={context}
        key={`${relayScope}:${signerScope}:${context.repoAddress}`}
        onResetWidth={repositoryProps.onResetWidth}
        onResizeStart={repositoryProps.onResizeStart}
        sharedHeaderBackdrop={sharedHeaderBackdrop}
        widthPx={repositoryProps.widthPx}
      />
    );
  }
  return (
    <ProjectRepositoryActionsPanel
      detached={detachedRepository}
      {...repositoryProps}
    />
  );
}
