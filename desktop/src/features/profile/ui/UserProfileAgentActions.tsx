import type { LucideIcon } from "lucide-react";
import { CopyPlus, Download, Trash2 } from "lucide-react";

import type { ManagedAgent } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";

export function UserProfileAgentActions({
  isPending,
  managedAgent,
  onDelete,
  onDuplicatePersona,
  onExportPersona,
  personaActionKey,
}: {
  isPending: boolean;
  managedAgent?: ManagedAgent;
  onDelete?: () => void;
  onDuplicatePersona?: () => void;
  onExportPersona?: () => void;
  personaActionKey?: string;
}) {
  const actionKey = managedAgent?.pubkey ?? "persona-draft";
  const personaKey = personaActionKey ?? actionKey;

  return (
    <section className="space-y-2">
      {onDuplicatePersona ? (
        <AgentActionRow
          disabled={isPending}
          icon={CopyPlus}
          label="Duplicate"
          onClick={onDuplicatePersona}
          testId={`user-profile-persona-duplicate-${personaKey}`}
        />
      ) : null}
      {onExportPersona ? (
        <AgentActionRow
          disabled={isPending}
          icon={Download}
          label="Export"
          onClick={onExportPersona}
          testId={`user-profile-persona-export-${personaKey}`}
        />
      ) : null}
      {onDelete ? (
        <AgentActionRow
          destructive
          disabled={isPending}
          icon={Trash2}
          label="Delete agent"
          onClick={onDelete}
          testId={`user-profile-agent-delete-${actionKey}`}
        />
      ) : null}
    </section>
  );
}

function AgentActionRow({
  destructive,
  disabled,
  icon: Icon,
  label,
  onClick,
  testId,
  trailing,
}: {
  destructive?: boolean;
  disabled?: boolean;
  icon: LucideIcon;
  label: string;
  onClick: () => void;
  testId: string;
  trailing?: string;
}) {
  return (
    <button
      className={cn(
        "flex w-full items-center gap-3 rounded-2xl bg-muted/20 px-4 py-2 text-left transition-colors hover:bg-muted/40 disabled:cursor-not-allowed disabled:opacity-50",
        destructive && "text-destructive hover:bg-destructive/10",
      )}
      data-testid={testId}
      disabled={disabled}
      onClick={onClick}
      type="button"
    >
      <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-muted/60">
        <Icon
          className={cn(
            "h-4 w-4",
            destructive ? "text-destructive" : "text-muted-foreground",
          )}
        />
      </span>
      <span className="min-w-0 flex-1 text-sm font-medium text-foreground">
        {label}
      </span>
      {trailing ? (
        <span className="text-sm capitalize text-muted-foreground">
          {trailing}
        </span>
      ) : null}
    </button>
  );
}
