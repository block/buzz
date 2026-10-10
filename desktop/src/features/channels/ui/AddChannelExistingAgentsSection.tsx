import { Check } from "lucide-react";

import type { RelayAgent } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";

function statusLabel(status: RelayAgent["status"]) {
  if (status === "online") return "Online";
  if (status === "away") return "Away";
  return "Offline";
}

function ExistingAgentRow({
  agent,
  disabled,
  inChannel,
  onToggle,
  selected,
}: {
  agent: RelayAgent;
  disabled: boolean;
  inChannel: boolean;
  onToggle: () => void;
  selected: boolean;
}) {
  return (
    <button
      aria-pressed={inChannel ? undefined : selected}
      className={cn(
        "flex w-full items-center gap-3 rounded-lg px-3 py-2.5 text-left transition-colors focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring",
        inChannel
          ? "cursor-default text-muted-foreground"
          : selected
            ? "bg-accent text-accent-foreground"
            : "hover:bg-accent/60",
        disabled && !inChannel && "cursor-not-allowed opacity-50",
      )}
      data-testid={`add-existing-agent-${agent.pubkey}`}
      disabled={disabled || inChannel}
      onClick={onToggle}
      type="button"
    >
      <ProfileAvatar
        avatarUrl={null}
        className="h-9 w-9 shrink-0 text-xs"
        iconClassName="h-5 w-5"
        label={agent.name}
      />
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium text-foreground">
          {agent.name}
        </span>
        <span className="block text-xs text-muted-foreground">
          {inChannel ? "Already in this channel" : statusLabel(agent.status)}
        </span>
      </span>
      {inChannel ? (
        <span className="inline-flex shrink-0 items-center gap-1 text-xs font-medium text-muted-foreground">
          <Check className="h-4 w-4" />
          In channel
        </span>
      ) : (
        <span
          aria-hidden
          className={cn(
            "flex h-5 w-5 shrink-0 items-center justify-center rounded border",
            selected
              ? "border-primary bg-primary text-primary-foreground"
              : "border-border bg-background",
          )}
        >
          {selected ? <Check className="h-3.5 w-3.5" /> : null}
        </span>
      )}
    </button>
  );
}

export function AddChannelExistingAgentsSection({
  agents,
  canToggleSelections,
  inChannelPubkeys,
  isLoading,
  onToggleAgent,
  selectedPubkeys,
}: {
  agents: RelayAgent[];
  canToggleSelections: boolean;
  inChannelPubkeys: ReadonlySet<string>;
  isLoading: boolean;
  onToggleAgent: (pubkey: string) => void;
  selectedPubkeys: readonly string[];
}) {
  if (!isLoading && agents.length === 0) {
    return null;
  }

  const available = agents.filter(
    (agent) => !inChannelPubkeys.has(normalizePubkey(agent.pubkey)),
  );
  const inChannel = agents.filter((agent) =>
    inChannelPubkeys.has(normalizePubkey(agent.pubkey)),
  );

  return (
    <div className="space-y-3" data-testid="add-existing-agents-section">
      <div className="px-3">
        <p className="text-xs font-medium text-muted-foreground">
          Existing agents
        </p>
        <p className="mt-1 text-xs text-muted-foreground">
          Add an agent that is already running on another computer. No local
          runtime is needed.
        </p>
      </div>

      {isLoading ? (
        <p className="px-3 text-sm text-muted-foreground">
          Loading existing agents…
        </p>
      ) : null}

      {available.length > 0 ? (
        <div className="space-y-1">
          {available.map((agent) => (
            <ExistingAgentRow
              agent={agent}
              disabled={!canToggleSelections}
              inChannel={false}
              key={agent.pubkey}
              onToggle={() => onToggleAgent(agent.pubkey)}
              selected={selectedPubkeys.includes(agent.pubkey)}
            />
          ))}
        </div>
      ) : null}

      {inChannel.length > 0 ? (
        <div className="space-y-1 border-t border-border pt-3">
          {inChannel.map((agent) => (
            <ExistingAgentRow
              agent={agent}
              disabled
              inChannel
              key={agent.pubkey}
              onToggle={() => undefined}
              selected={false}
            />
          ))}
        </div>
      ) : null}
    </div>
  );
}
