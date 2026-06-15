import type { ManagedAgent, PresenceLookup } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { ManagedAgentRow } from "./ManagedAgentRow";

export type AgentGroupRowsProps = {
  agents: ManagedAgent[];
  channelIdToName: Record<string, string>;
  channelsByPubkey: Record<string, { id: string; name: string }[]>;
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
  channelIdToName,
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
    <div className="divide-y divide-border/50 border-t border-border/50">
      {agents.map((agent) => (
        <ManagedAgentRow
          agent={agent}
          channelIdToName={channelIdToName}
          channelNames={channelsByPubkey[normalizePubkey(agent.pubkey)] ?? []}
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
