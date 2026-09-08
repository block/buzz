import { useState } from "react";

import { ConversationComposerDock } from "@/features/conversation/ui/ConversationComposerDock";
import type { AgentTurn } from "@/features/agent-activity/types";

const WORKING_TURN: AgentTurn = {
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

/**
 * Product-composition documentation for the Conversation Composer.
 *
 * This is not a shared UI component API. It renders the actual dock and
 * composer so the page remains a visual contract for the product composition
 * as its real authoring capabilities arrive.
 */
function ComposerSpecimen({ turns = [] }: { turns?: AgentTurn[] }) {
  const [draft, setDraft] = useState("");

  return (
    <div className="composer-page-specimen">
      <ConversationComposerDock
        draft={draft}
        onDraftChange={setDraft}
        onSend={async () => undefined}
        turns={turns}
      />
    </div>
  );
}

export function ComposerPage() {
  return (
    <>
      <header className="component-page-heading">
        <h1 className="text-title text-primary">Composer</h1>
        <p className="max-w-prose text-body text-secondary">
          The conversation authoring surface. Context arrives above when it
          changes the contribution; people, agents, and channels stay inline in
          the authored text; activity makes room beneath without moving the
          Composer’s top edge.
        </p>
      </header>

      <div className="component-specimen-stack">
        <section className="component-specimen-group">
          <h2 className="text-body-sm text-tertiary">Writing</h2>
          <ComposerSpecimen />
        </section>

        <section className="component-specimen-group">
          <h2 className="text-body-sm text-tertiary">
            Working alongside an agent
          </h2>
          <p className="max-w-prose text-body-sm text-secondary">
            The activity rail takes only its own height plus a 4px gap from the
            Composer’s bottom edge. The Composer keeps its top edge and remains
            usable while an agent works.
          </p>
          <ComposerSpecimen turns={[WORKING_TURN]} />
        </section>
      </div>
    </>
  );
}
