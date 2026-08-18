import * as React from "react";

import type { ManagedAgent } from "@/shared/api/types";
import {
  AGENT_MANAGEMENT_REQUEST,
  createDefaultNxtlinqPolicyDraft,
  type AgentManagementNxtlinqSetupRequest,
} from "../agentManagement";
import { NxtlinqSetupReviewDialog } from "./NxtlinqSetupReviewDialog";

export function useNxtlinqDirectSetup(
  agent: ManagedAgent,
  workingDirectoryOverride: string,
  onReviewClosed: () => void,
) {
  const [isOpen, setIsOpen] = React.useState(false);
  const projectRoot =
    workingDirectoryOverride.trim() || agent.workingDirectory || "";
  const request = React.useMemo<AgentManagementNxtlinqSetupRequest>(
    () => ({
      type: AGENT_MANAGEMENT_REQUEST,
      action: "nxtlinq_setup",
      requestId: `ui-${agent.pubkey}`,
      request: {
        channelId: "desktop-owner-review",
        projectRoot,
        explanation:
          "Conservative Buzz baseline: read ordinary project documentation and source, exclude secrets and signing material, and connect to the bundled Buzz MCP server without granting tool invocation.",
        policy: createDefaultNxtlinqPolicyDraft(agent.name),
      },
    }),
    [agent.name, agent.pubkey, projectRoot],
  );

  const dialog = isOpen ? (
    <NxtlinqSetupReviewDialog
      agent={agent}
      onOpenChange={(next) => {
        setIsOpen(next);
        if (!next) onReviewClosed();
      }}
      proposalSource="default"
      request={request}
    />
  ) : null;

  return { dialog, isOpen, open: () => setIsOpen(true) };
}
