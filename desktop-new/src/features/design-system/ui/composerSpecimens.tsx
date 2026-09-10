import { Switch } from "@/shared/ui/Switch";
import { useState } from "react";

import type { AgentTurn } from "@/features/agent-activity/types";
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
        autoFocus={false}
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
  const [working, setWorking] = useState(true);
  return (
    <div className="component-specimen-stack">
      <Switch
        checked={working}
        onCheckedChange={setWorking}
        label="Show agent activity"
      />
      <ComposerSpecimen turns={working ? [WORKING_TURN] : []} />
    </div>
  );
}

export function MessageComposerSpecimen() {
  const [draft, setDraft] = useState("");
  return (
    <MessageComposer
      autoFocus={false}
      draft={draft}
      onDraftChange={setDraft}
      onSend={async () => undefined}
      placeholder="Reply in this session"
    />
  );
}

/** The formatting bar is never useful alone: mount it through its real composer. */
export function FormattingBarSpecimen() {
  const [draft, setDraft] = useState("Select some text, then format it.");
  return (
    <MessageComposer
      autoFocus={false}
      defaultFormattingOpen
      draft={draft}
      onDraftChange={setDraft}
      onSend={async () => undefined}
      placeholder="Reply in this session"
    />
  );
}
