import { Bot, Check } from "lucide-react";

import type { AgentPersona } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { promptPreview } from "@/shared/lib/promptPreview";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/shared/ui/tooltip";

type SelectionChipButtonProps = {
  avatarUrl?: string | null;
  disabled: boolean;
  label: string;
  onClick: () => void;
  selected: boolean;
  children: React.ReactNode;
};

function SelectionChipButton({
  avatarUrl,
  disabled,
  label,
  onClick,
  selected,
  children,
}: SelectionChipButtonProps) {
  const showAvatar = avatarUrl !== undefined;

  return (
    <button
      aria-pressed={selected}
      className={cn(
        "inline-flex min-h-9 items-center gap-2 rounded-full border py-1.5 text-sm font-medium transition-colors focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring",
        showAvatar ? "pl-1.5 pr-3" : "px-3",
        selected
          ? "border-primary bg-primary/10 text-foreground"
          : "border-border/80 bg-background/60 text-muted-foreground hover:bg-accent hover:text-accent-foreground",
        disabled && "cursor-not-allowed opacity-50",
      )}
      disabled={disabled}
      onClick={onClick}
      type="button"
    >
      {showAvatar ? (
        <ProfileAvatar
          avatarUrl={avatarUrl}
          className={cn(
            "h-6 w-6 text-[10px]",
            selected
              ? "bg-primary/20 text-primary ring-1 ring-primary/20"
              : "bg-background/80 text-muted-foreground ring-1 ring-border/70",
          )}
          iconClassName="h-3.5 w-3.5"
          label={label}
        />
      ) : null}
      {children}
    </button>
  );
}

function RuntimeBadge({
  label,
  isOverridden,
}: {
  label: string;
  isOverridden: boolean;
}) {
  return (
    <span
      className={cn(
        "inline-flex items-center rounded-full px-1.5 py-0.5 text-[10px] font-medium leading-none",
        isOverridden
          ? "bg-warning/15 text-warning"
          : "bg-muted/60 text-muted-foreground",
      )}
    >
      {label}
    </span>
  );
}

type AddChannelBotPersonasSectionProps = {
  canToggleSelections: boolean;
  /** Map of personaId → effective runtime label for badge display */
  effectiveRuntimes?: ReadonlyMap<
    string,
    { label: string; isOverridden: boolean }
  >;
  inChannelPersonaIds?: ReadonlySet<string>;
  includeGeneric: boolean;
  isLoading: boolean;
  onToggleGeneric: () => void;
  onTogglePersona: (personaId: string) => void;
  personas: AgentPersona[];
  selectedPersonaIds: readonly string[];
  /** Whether to show the "Generic" chip. Defaults to true. */
  showGeneric?: boolean;
};

export function AddChannelBotPersonasSection({
  canToggleSelections,
  effectiveRuntimes,
  inChannelPersonaIds,
  includeGeneric,
  isLoading,
  onToggleGeneric,
  onTogglePersona,
  personas,
  selectedPersonaIds,
  showGeneric = true,
}: AddChannelBotPersonasSectionProps) {
  return (
    <div className="space-y-3">
      <div className="space-y-3">
        <div>
          <div className="text-sm font-medium">Personas</div>
          <p className="text-xs text-muted-foreground">
            Toggle as many as you want. Each selected persona is added as its
            own agent. Hover a persona to preview its role.
          </p>
        </div>

        <TooltipProvider delayDuration={150}>
          <div className="flex flex-wrap gap-2">
            {showGeneric ? (
              <Tooltip>
                <TooltipTrigger asChild>
                  <div>
                    <SelectionChipButton
                      disabled={!canToggleSelections}
                      label="Generic"
                      onClick={onToggleGeneric}
                      selected={includeGeneric}
                    >
                      <Bot
                        className={cn(
                          "h-4 w-4",
                          includeGeneric ? "text-primary" : "text-current",
                        )}
                      />
                      Generic
                    </SelectionChipButton>
                  </div>
                </TooltipTrigger>
                <TooltipContent className="max-w-xs text-left">
                  Add one custom agent with a channel-specific name and prompt.
                </TooltipContent>
              </Tooltip>
            ) : null}
            {personas.map((persona) => {
              const isSelected = selectedPersonaIds.includes(persona.id);
              const isInChannel = inChannelPersonaIds?.has(persona.id) ?? false;
              const runtimeBadge = effectiveRuntimes?.get(persona.id);
              return (
                <Tooltip key={persona.id}>
                  <TooltipTrigger asChild>
                    <div>
                      <SelectionChipButton
                        avatarUrl={persona.avatarUrl}
                        disabled={!canToggleSelections || isInChannel}
                        label={persona.displayName}
                        onClick={() => onTogglePersona(persona.id)}
                        selected={isSelected}
                      >
                        {persona.displayName}
                        {runtimeBadge ? (
                          <RuntimeBadge {...runtimeBadge} />
                        ) : null}
                        {isInChannel ? (
                          <span
                            className={cn(
                              "inline-flex items-center gap-0.5 rounded-full px-1.5 py-0.5 text-[10px] font-medium leading-none",
                              isSelected
                                ? "bg-primary/15 text-primary"
                                : "bg-muted/60 text-muted-foreground",
                            )}
                          >
                            <Check className="h-2.5 w-2.5" />
                            In channel
                          </span>
                        ) : null}
                      </SelectionChipButton>
                    </div>
                  </TooltipTrigger>
                  <TooltipContent className="max-w-xs text-left">
                    <div className="space-y-2">
                      <div className="flex items-center gap-2">
                        <ProfileAvatar
                          avatarUrl={persona.avatarUrl}
                          className="h-7 w-7 text-[10px] bg-primary-foreground/20 text-primary-foreground"
                          iconClassName="h-3.5 w-3.5"
                          label={persona.displayName}
                        />
                        <p className="font-medium">{persona.displayName}</p>
                      </div>
                      {isInChannel ? (
                        <p className="text-[11px] font-medium text-emerald-300">
                          ✓ Already in this channel
                        </p>
                      ) : null}
                      <p className="text-[11px] text-primary-foreground">
                        {promptPreview(persona.systemPrompt)}
                      </p>
                    </div>
                  </TooltipContent>
                </Tooltip>
              );
            })}
          </div>
        </TooltipProvider>

        {isLoading ? (
          <p className="text-xs text-muted-foreground">Loading personas...</p>
        ) : null}
      </div>
    </div>
  );
}
