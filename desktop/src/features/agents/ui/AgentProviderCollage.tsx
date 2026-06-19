import type { CSSProperties } from "react";

import { IdentityInitialsAvatar } from "./IdentityInitialsAvatar";

type ClusterLayout = {
  avatarStyle: CSSProperties;
};

type AgentProviderCollageProps = {
  label: string;
};

const CLUSTER_CENTER_TOP_PERCENT = 50;
const AGENT_INITIAL_AVATAR_SIZE = 152;

export function AgentProviderCollage({ label }: AgentProviderCollageProps) {
  const { avatarStyle } = buildClusterPoints();

  return (
    <div className="relative h-full w-full overflow-hidden px-4 pt-0 pb-12">
      <div className="relative isolate h-full w-full min-w-0">
        <div
          className="-translate-x-1/2 -translate-y-1/2 absolute z-10 h-[152px] w-[152px]"
          data-agent-cluster-item="avatar"
          style={avatarStyle}
        >
          <IdentityInitialsAvatar
            colorSeed={label}
            label={label}
            size={AGENT_INITIAL_AVATAR_SIZE}
          />
        </div>
      </div>
    </div>
  );
}

function buildClusterPoints(): ClusterLayout {
  return {
    avatarStyle: {
      left: "50%",
      top: `${CLUSTER_CENTER_TOP_PERCENT}%`,
    },
  };
}
