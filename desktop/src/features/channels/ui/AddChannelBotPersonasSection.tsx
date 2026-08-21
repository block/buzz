import { Check, Plus } from "lucide-react";

import type { AgentPersona } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";

function AgentRow({
  persona,
  selected,
  disabled,
  inChannel,
  onToggle,
}: {
  persona: AgentPersona;
  selected: boolean;
  disabled: boolean;
  inChannel: boolean;
  onToggle: () => void;
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
      disabled={disabled || inChannel}
      onClick={onToggle}
      type="button"
    >
      <ProfileAvatar
        avatarUrl={persona.avatarUrl}
        className="h-9 w-9 shrink-0 text-xs"
        iconClassName="h-5 w-5"
        label={persona.displayName}
      />
      <span className="min-w-0 flex-1 truncate text-sm font-medium text-foreground">
        {persona.displayName}
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

function CreateAgentRow({ onCreateAgent }: { onCreateAgent: () => void }) {
  return (
    <button
      className="flex w-full items-center gap-3 rounded-lg border border-border bg-card px-3 py-3 text-left transition-colors hover:bg-accent/50 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
      data-testid="add-channel-create-agent"
      onClick={onCreateAgent}
      type="button"
    >
      <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-primary text-primary-foreground">
        <Plus className="h-5 w-5" />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block text-sm font-medium text-foreground">
          Create a new agent
        </span>
        <span className="block text-xs text-muted-foreground">
          Give it a name, purpose, and instructions.
        </span>
      </span>
    </button>
  );
}

type AddChannelBotPersonasSectionProps = {
  canToggleSelections: boolean;
  inChannelPersonaIds?: ReadonlySet<string>;
  isLoading: boolean;
  onCreateAgent?: () => void;
  onTogglePersona: (personaId: string) => void;
  personas: AgentPersona[];
  selectedPersonaIds: readonly string[];
  // Legacy no-op props retained for the channel-template selector. Generic
  // remains supported there but is no longer exposed by channel add flows.
  includeGeneric?: boolean;
  onToggleGeneric?: () => void;
  showGeneric?: boolean;
};

export function AddChannelBotPersonasSection({
  canToggleSelections,
  inChannelPersonaIds,
  isLoading,
  onCreateAgent,
  onTogglePersona,
  personas,
  selectedPersonaIds,
}: AddChannelBotPersonasSectionProps) {
  const available = personas.filter(
    (persona) => !inChannelPersonaIds?.has(persona.id),
  );
  const inChannel = personas.filter((persona) =>
    inChannelPersonaIds?.has(persona.id),
  );

  return (
    <div className="space-y-4">
      {onCreateAgent && available.length === 0 ? (
        <CreateAgentRow onCreateAgent={onCreateAgent} />
      ) : null}

      {isLoading ? (
        <p className="px-3 text-sm text-muted-foreground">
          Loading your agents…
        </p>
      ) : null}

      {!isLoading && available.length > 0 ? (
        <div className="space-y-1">
          <div className="px-3 pb-1 text-xs font-medium text-muted-foreground">
            Your agents
          </div>
          {available.map((persona) => (
            <AgentRow
              disabled={!canToggleSelections}
              inChannel={false}
              key={persona.id}
              onToggle={() => onTogglePersona(persona.id)}
              persona={persona}
              selected={selectedPersonaIds.includes(persona.id)}
            />
          ))}
        </div>
      ) : null}

      {!isLoading && available.length === 0 && inChannel.length > 0 ? (
        <p className="px-3 text-sm text-muted-foreground">
          All of your agents are already in this channel.
        </p>
      ) : null}

      {onCreateAgent && available.length > 0 ? (
        <CreateAgentRow onCreateAgent={onCreateAgent} />
      ) : null}

      {!isLoading && inChannel.length > 0 ? (
        <div className="space-y-1 border-t border-border pt-3">
          <div className="px-3 pb-1 text-xs font-medium text-muted-foreground">
            In this channel
          </div>
          {inChannel.map((persona) => (
            <AgentRow
              disabled
              inChannel
              key={persona.id}
              onToggle={() => undefined}
              persona={persona}
              selected={false}
            />
          ))}
        </div>
      ) : null}
    </div>
  );
}
