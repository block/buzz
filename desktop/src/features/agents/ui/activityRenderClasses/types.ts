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
};

export type ActivityRenderClassPresenter =
  React.ComponentType<ActivityRenderClassItemProps>;
