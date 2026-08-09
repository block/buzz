import * as React from "react";
import { Anchor, Circle } from "lucide-react";

import { useOpenAgentActivity } from "@/features/agents/useOpenAgentActivity";
import type { ShipRoomId } from "../domain/shipLayout";
import { useLivingShipAgents } from "../hooks/useLivingShipAgents";
import { LivingShipCanvas } from "./LivingShipCanvas";
import { LivingShipDetails } from "./LivingShipDetails";
import "../livingShip.css";

export function LivingShipScreen() {
  const { agents, errorMessage, isLoading } = useLivingShipAgents();
  const { canOpenAgentActivity, openAgentActivity } = useOpenAgentActivity();
  const [selectedAgentPubkey, setSelectedAgentPubkey] = React.useState<
    string | null
  >(null);
  const [selectedRoomId, setSelectedRoomId] = React.useState<ShipRoomId | null>(
    null,
  );
  const selectedAgent =
    agents.find((agent) => agent.pubkey === selectedAgentPubkey) ?? null;

  if (isLoading) {
    return (
      <div className="living-ship-empty" data-testid="living-ship-loading">
        Preparing the ship…
      </div>
    );
  }

  return (
    <main className="living-ship-screen">
      <header className="living-ship-header">
        <div>
          <div className="living-ship-kicker">
            <Anchor aria-hidden="true" className="h-4 w-4" /> Command Adviser
          </div>
          <h1>HMAS Supply · Living Ship</h1>
          <p>
            Where your command team and support agents are working,
            collaborating or standing by.
          </p>
        </div>
        <fieldset
          aria-label="Agent state legend"
          className="living-ship-legend"
        >
          <span>
            <Circle className="living-ship-dot is-working" /> Working
          </span>
          <span>
            <Circle className="living-ship-dot is-collaborating" />{" "}
            Collaborating
          </span>
          <span>
            <Circle className="living-ship-dot is-idle" /> Wardroom
          </span>
          <span>
            <Circle className="living-ship-dot is-offline" /> Not aboard
          </span>
        </fieldset>
      </header>
      {errorMessage ? (
        <div className="living-ship-error" role="alert">
          {errorMessage}
        </div>
      ) : null}
      {agents.length === 0 ? (
        <div className="living-ship-empty">
          No Command Adviser or support agents are configured on this ship yet.
        </div>
      ) : (
        <div className="living-ship-workspace">
          <LivingShipCanvas
            agents={agents}
            onSelectAgent={(pubkey) => {
              setSelectedAgentPubkey(pubkey);
              setSelectedRoomId(null);
            }}
            onSelectRoom={(roomId) => {
              setSelectedRoomId(roomId);
              setSelectedAgentPubkey(null);
            }}
            selectedAgentPubkey={selectedAgentPubkey}
            selectedRoomId={selectedRoomId}
          />
          <LivingShipDetails
            agents={agents}
            canOpenActivity={canOpenAgentActivity(selectedAgent?.pubkey)}
            onOpenActivity={(agent) => {
              openAgentActivity(agent.pubkey, { channelId: agent.channelId });
            }}
            selectedAgent={selectedAgent}
            selectedRoomId={selectedRoomId}
          />
        </div>
      )}
    </main>
  );
}
