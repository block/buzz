import type { ManagedAgent, PresenceLookup } from "@/shared/api/types";
import { ManagedAgentRow } from "./ManagedAgentRow";

export type AgentGroupRowsProps = {
  agents: ManagedAgent[];
  channelsByPubkey: Record<string, string[]>;
  isActionPending: boolean;
  logContent: string | null;
  logError: Error | null;
  logLoading: boolean;
  personaLabelsById: Record<string, string>;
  presenceLoaded: boolean;
  presenceLookup: PresenceLookup;
  selectedLogAgentPubkey: string | null;
  onAddToChannel: (agent: ManagedAgent) => void;
  onDelete: (pubkey: string) => void;
  onSelectLogAgent: (pubkey: string | null) => void;
  onStart: (pubkey: string) => void;
  onStop: (pubkey: string) => void;
  onToggleStartOnAppLaunch: (pubkey: string, startOnAppLaunch: boolean) => void;
};

export function AgentGroupRows({
  agents,
  channelsByPubkey,
  isActionPending,
  logContent,
  logError,
  logLoading,
  personaLabelsById,
  presenceLoaded,
  presenceLookup,
  selectedLogAgentPubkey,
  onAddToChannel,
  onDelete,
  onSelectLogAgent,
  onStart,
  onStop,
  onToggleStartOnAppLaunch,
}: AgentGroupRowsProps) {
  return (
    <div className="space-y-2 border-t border-border/50 px-3 py-2">
      {agents.map((agent) => (
        <ManagedAgentRow
          agent={agent}
          channelNames={channelsByPubkey[agent.pubkey] ?? []}
          isActionPending={isActionPending}
          isLogSelected={selectedLogAgentPubkey === agent.pubkey}
          key={agent.pubkey}
          logContent={
            selectedLogAgentPubkey === agent.pubkey ? logContent : null
          }
          logError={selectedLogAgentPubkey === agent.pubkey ? logError : null}
          logLoading={selectedLogAgentPubkey === agent.pubkey && logLoading}
          personaLabelsById={personaLabelsById}
          presenceLoaded={presenceLoaded}
          presenceLookup={presenceLookup}
          onAddToChannel={onAddToChannel}
          onDelete={onDelete}
          onSelectLogAgent={onSelectLogAgent}
          onStart={onStart}
          onStop={onStop}
          onToggleStartOnAppLaunch={onToggleStartOnAppLaunch}
        />
      ))}
    </div>
  );
}
