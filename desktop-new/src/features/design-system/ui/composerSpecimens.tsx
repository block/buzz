import { useState } from "react";

import type { AgentTurn } from "@/features/agent-activity/types";
import { AgentActivityRail } from "@/features/agent-activity/ui/AgentActivityRail";
import { ConversationComposerDock } from "@/features/conversation/ui/ConversationComposerDock";
import { MessageComposer } from "@/features/composer/ui/MessageComposer";

export const WORKING_TURN: AgentTurn = {
  key: "design:vogue:session:turn",
  agentPubkey: "vogue",
  agentName: "Vogue",
  sessionId: "session",
  turnId: "turn",
  channelId: "design",
  status: "running",
  items: [
    {
      id: "review",
      label: "Reviewing the interaction plan",
      status: "running",
      timestamp: "2026-09-08T03:00:00.000Z",
    },
  ],
};

export function ComposerSpecimen({
  turns = [],
  responseControl,
}: {
  turns?: AgentTurn[];
  responseControl?: { state: "responding" | "stopping" };
}) {
  const [draft, setDraft] = useState("");

  return (
    <div className="composer-page-specimen">
      <ConversationComposerDock
        draft={draft}
        onDraftChange={setDraft}
        onSend={async () => undefined}
        placeholder="Reply in this session"
        responseControl={responseControl}
        turns={turns}
      />
    </div>
  );
}

export function ActivityRailSpecimen() {
  return <AgentActivityRail turns={[WORKING_TURN]} />;
}

export function MessageComposerSpecimen() {
  const [draft, setDraft] = useState("");
  return (
    <MessageComposer
      draft={draft}
      onDraftChange={setDraft}
      onSend={async () => undefined}
      placeholder="Reply in this session"
    />
  );
}
