import type * as React from "react";

import shipScene from "../assets/hmas-supply-living-ship.png";
import agentSprites from "../assets/agent-sprites.png";
import { SHIP_ROOMS, type ShipRoomId } from "../domain/shipLayout";
import type { LivingShipAgentPresentation } from "../domain/shipProjection";

type CanvasStyle = React.CSSProperties & Record<`--${string}`, number | string>;

const SHIP_SCENE_WIDTH = 1754;
const SHIP_SCENE_HEIGHT = 896;

function scenePercent(value: number, extent: number) {
  return `${((value / extent) * 100).toFixed(6)}%`;
}

type LivingShipCanvasProps = {
  agents: readonly LivingShipAgentPresentation[];
  selectedAgentPubkey: string | null;
  selectedRoomId: ShipRoomId | null;
  onSelectAgent: (pubkey: string) => void;
  onSelectRoom: (roomId: ShipRoomId) => void;
};

function agentPosition(
  agent: LivingShipAgentPresentation,
  agents: readonly LivingShipAgentPresentation[],
) {
  const peers = agents.filter(
    (candidate) => candidate.locationId === agent.locationId,
  );
  const index = peers.findIndex(
    (candidate) => candidate.pubkey === agent.pubkey,
  );
  if (agent.locationId === "personnel-strip") {
    return { x: 52 + index * 66, y: 102 };
  }
  const room = SHIP_ROOMS.find(
    (candidate) => candidate.id === agent.locationId,
  );
  if (!room) return { x: 52, y: 54 };
  const spacing = Math.min(54, (room.width - 44) / Math.max(peers.length, 1));
  return {
    x: room.x + 12 + index * spacing,
    y: room.y + Math.max(0, room.height - 78),
  };
}

function agentAriaLabel(agent: LivingShipAgentPresentation) {
  if (agent.locationId === "personnel-strip") {
    return `Select ${agent.label}, not aboard`;
  }
  const room = SHIP_ROOMS.find(
    (candidate) => candidate.id === agent.locationId,
  );
  return `Select ${agent.label} in ${room?.label ?? "ship"}`;
}

export function LivingShipCanvas({
  agents,
  selectedAgentPubkey,
  selectedRoomId,
  onSelectAgent,
  onSelectRoom,
}: LivingShipCanvasProps) {
  const canvasStyle: CanvasStyle = {
    "--ship-scene-width": SHIP_SCENE_WIDTH,
    "--ship-scene-height": SHIP_SCENE_HEIGHT,
  };
  return (
    <section
      aria-label="HMAS Supply living ship workspace"
      className="living-ship-canvas"
      data-testid="living-ship-canvas"
      style={canvasStyle}
    >
      <img
        alt="Pixel-art side elevation of HMAS Supply with visible workspaces"
        className="living-ship-scene"
        draggable={false}
        src={shipScene}
      />
      <div className="living-ship-personnel-strip">
        <span className="living-ship-personnel-title">
          Personnel not aboard
        </span>
        <span className="living-ship-personnel-subtitle">
          Stopped, waking or unavailable
        </span>
      </div>
      {SHIP_ROOMS.map((room) => {
        const occupants = agents.filter(
          (agent) => agent.locationId === room.id,
        );
        const style: CanvasStyle = {
          "--room-x": scenePercent(room.x, SHIP_SCENE_WIDTH),
          "--room-y": scenePercent(room.y, SHIP_SCENE_HEIGHT),
          "--room-width": scenePercent(room.width, SHIP_SCENE_WIDTH),
          "--room-height": scenePercent(room.height, SHIP_SCENE_HEIGHT),
        };
        return (
          <button
            aria-label={`Open ${room.label} workspace, ${occupants.length} occupant${occupants.length === 1 ? "" : "s"}`}
            className="living-ship-room"
            data-occupied={occupants.length > 0 ? "true" : "false"}
            data-room-id={room.id}
            data-selected={selectedRoomId === room.id ? "true" : "false"}
            key={room.id}
            onClick={() => onSelectRoom(room.id)}
            style={style}
            type="button"
          >
            <span>{room.label}</span>
          </button>
        );
      })}
      {agents.map((agent) => {
        const position = agentPosition(agent, agents);
        const style: CanvasStyle = {
          "--agent-x": scenePercent(position.x, SHIP_SCENE_WIDTH),
          "--agent-y": scenePercent(position.y, SHIP_SCENE_HEIGHT),
          "--sprite-position": `${(agent.spriteColumn / 7) * 100}%`,
        };
        const state =
          agent.lifecycle !== "online"
            ? "offline"
            : agent.collaborationId
              ? "collaborating"
              : agent.working
                ? "working"
                : "idle";
        return (
          <button
            aria-label={agentAriaLabel(agent)}
            className="living-ship-agent"
            data-selected={
              selectedAgentPubkey === agent.pubkey ? "true" : "false"
            }
            data-state={state}
            key={agent.pubkey}
            onClick={() => onSelectAgent(agent.pubkey)}
            style={style}
            type="button"
          >
            <span
              aria-hidden="true"
              className="living-ship-agent-sprite"
              style={{ backgroundImage: `url(${agentSprites})` }}
            />
            <span className="living-ship-agent-label">{agent.shortLabel}</span>
          </button>
        );
      })}
    </section>
  );
}
