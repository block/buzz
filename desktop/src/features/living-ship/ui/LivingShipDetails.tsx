import { ExternalLink, Radio, Users } from "lucide-react";

import { SHIP_ROOMS, type ShipRoomId } from "../domain/shipLayout";
import type { LivingShipAgentPresentation } from "../domain/shipProjection";

type LivingShipDetailsProps = {
  agents: readonly LivingShipAgentPresentation[];
  selectedAgent: LivingShipAgentPresentation | null;
  selectedRoomId: ShipRoomId | null;
  canOpenActivity: boolean;
  onOpenActivity: (agent: LivingShipAgentPresentation) => void;
};

function roomLabel(roomId: LivingShipAgentPresentation["locationId"]) {
  if (roomId === "personnel-strip") return "Not aboard";
  return SHIP_ROOMS.find((room) => room.id === roomId)?.label ?? roomId;
}

function stateLabel(agent: LivingShipAgentPresentation) {
  if (agent.lifecycle !== "online") return "Not aboard";
  if (agent.collaborationId) return "Collaborating";
  if (agent.working) return "Working";
  return "Chilling in the Wardroom";
}

export function LivingShipDetails({
  agents,
  selectedAgent,
  selectedRoomId,
  canOpenActivity,
  onOpenActivity,
}: LivingShipDetailsProps) {
  if (selectedAgent) {
    const collaborators = agents.filter((agent) =>
      selectedAgent.collaboratorPubkeys.includes(agent.pubkey),
    );
    return (
      <aside className="living-ship-details" data-testid="ship-agent-details">
        <div className="living-ship-details-eyebrow">
          {stateLabel(selectedAgent)}
        </div>
        <h2>{selectedAgent.label}</h2>
        <dl>
          <div>
            <dt>Workspace</dt>
            <dd>{roomLabel(selectedAgent.locationId)}</dd>
          </div>
          <div>
            <dt>Channel</dt>
            <dd>{selectedAgent.channelName ?? "No active channel"}</dd>
          </div>
          <div>
            <dt>Task</dt>
            <dd>
              {selectedAgent.taskSummary ??
                (selectedAgent.working ? "Active agent turn" : "Standing by")}
            </dd>
          </div>
        </dl>
        {collaborators.length > 0 ? (
          <div className="living-ship-collaborators">
            <Users aria-hidden="true" className="h-4 w-4" />
            With {collaborators.map((agent) => agent.label).join(", ")}
          </div>
        ) : null}
        <button
          className="living-ship-activity-button"
          disabled={!canOpenActivity}
          onClick={() => onOpenActivity(selectedAgent)}
          type="button"
        >
          <Radio aria-hidden="true" className="h-4 w-4" />
          Open activity
          <ExternalLink aria-hidden="true" className="h-3.5 w-3.5" />
        </button>
      </aside>
    );
  }

  if (selectedRoomId) {
    const room = SHIP_ROOMS.find(
      (candidate) => candidate.id === selectedRoomId,
    );
    const occupants = agents.filter(
      (agent) => agent.locationId === selectedRoomId,
    );
    return (
      <aside className="living-ship-details" data-testid="ship-room-details">
        <div className="living-ship-details-eyebrow">Workspace</div>
        <h2>{room?.label}</h2>
        <p>
          {occupants.length === 0
            ? "No advisers are currently in this workspace."
            : occupants.map((agent) => agent.label).join(", ")}
        </p>
      </aside>
    );
  }

  return (
    <aside className="living-ship-details living-ship-details-hint">
      Select an adviser or compartment to inspect live activity.
    </aside>
  );
}
