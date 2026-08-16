import { Check } from "lucide-react";

import type {
  RelayAgent,
  UserProfileSummary,
} from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";

type AddChannelBotRelayAgentsSectionProps = {
  canToggleSelections: boolean;
  inChannelPubkeys: ReadonlySet<string>;
  isLoading: boolean;
  onToggleAgent: (pubkey: string) => void;
  profiles: Record<string, UserProfileSummary>;
  relayAgents: RelayAgent[];
  selectedPubkeys: readonly string[];
};

export function AddChannelBotRelayAgentsSection({
  canToggleSelections,
  inChannelPubkeys,
  isLoading,
  onToggleAgent,
  profiles,
  relayAgents,
  selectedPubkeys,
}: AddChannelBotRelayAgentsSectionProps) {
  const available = relayAgents
    .filter((agent) => !inChannelPubkeys.has(normalizePubkey(agent.pubkey)))
    .sort((left, right) => left.name.localeCompare(right.name));

  if (!isLoading && available.length === 0) return null;

  return (
    <div className="space-y-1 border-t border-border pt-3">
      <div className="px-3 pb-1 text-xs font-medium text-muted-foreground">
        Available agents
      </div>
      {isLoading ? (
        <p className="px-3 text-sm text-muted-foreground">
          Loading available agents…
        </p>
      ) : null}
      {available.map((agent) => {
        const pubkey = normalizePubkey(agent.pubkey);
        const selected = selectedPubkeys.includes(pubkey);
        const profile = profiles[pubkey];
        const label = profile?.displayName?.trim() || agent.name;
        return (
          <button
            aria-pressed={selected}
            className={cn(
              "flex w-full items-center gap-3 rounded-lg px-3 py-2.5 text-left transition-colors focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring",
              selected
                ? "bg-accent text-accent-foreground"
                : "hover:bg-accent/60",
              !canToggleSelections && "cursor-not-allowed opacity-50",
            )}
            data-testid={`add-channel-relay-agent-${pubkey}`}
            disabled={!canToggleSelections}
            key={pubkey}
            onClick={() => onToggleAgent(pubkey)}
            type="button"
          >
            <ProfileAvatar
              avatarUrl={profile?.avatarUrl ?? null}
              className="h-9 w-9 shrink-0 text-xs"
              iconClassName="h-5 w-5"
              label={label}
            />
            <span className="min-w-0 flex-1 truncate text-sm font-medium text-foreground">
              {label}
            </span>
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
          </button>
        );
      })}
    </div>
  );
}
