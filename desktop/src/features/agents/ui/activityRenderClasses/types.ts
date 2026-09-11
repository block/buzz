import type * as React from "react";

import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { TranscriptItem } from "../agentSessionTypes";

export type AgentTranscriptIdentityProps = {
  agentAvatarUrl: string | null;
  agentName: string;
  agentPubkey: string;
};

export type ActivityRenderClassItemProps = AgentTranscriptIdentityProps & {
  item: TranscriptItem;
  profiles?: UserProfileLookup;
  /** Controlled detail expansion for rows whose state must survive regrouping. */
  expanded?: boolean;
  onExpansionChange?: (expanded: boolean) => void;
};

export type ActivityRenderClassPresenter =
  React.ComponentType<ActivityRenderClassItemProps>;
