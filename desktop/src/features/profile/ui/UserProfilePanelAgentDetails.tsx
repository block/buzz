import { ChevronRight, Cpu, MessageSquare } from "lucide-react";

import type { ManagedAgent } from "@/shared/api/types";
import { Markdown } from "@/shared/ui/markdown";
import {
  type ProfileField,
  ProfileFieldRows,
} from "@/features/profile/ui/UserProfilePanelFields";

export const AGENT_DETAILS_FIELD_LABELS = new Set([
  "Runtime",
  "ACP command",
  "MCP command",
]);

export function AgentConfigurationFocusedView({
  fields,
  managedAgent,
  modelLabel,
}: {
  fields: ProfileField[];
  managedAgent: ManagedAgent | undefined;
  modelLabel: string;
}) {
  const runtimeConfigurationFields = fields.filter((field) =>
    AGENT_DETAILS_FIELD_LABELS.has(field.label),
  );

  return (
    <div className="pt-4">
      <AgentConfigurationRows
        fields={runtimeConfigurationFields}
        managedAgent={managedAgent}
        modelLabel={modelLabel}
        showModel={true}
      />
    </div>
  );
}

function AgentConfigurationRows({
  fields,
  instruction,
  managedAgent,
  modelLabel,
  showInstructionPlaceholder,
  showModel,
}: {
  fields: ProfileField[];
  instruction?: string | null;
  managedAgent: ManagedAgent | undefined;
  modelLabel: string;
  showInstructionPlaceholder?: boolean;
  showModel: boolean;
}) {
  const hasRows = hasAgentConfigurationRows({
    fields,
    instruction,
    managedAgent,
    modelLabel,
    showInstructionPlaceholder,
    showModel,
  });

  if (!hasRows) {
    return null;
  }

  return (
    <div className="overflow-hidden rounded-2xl bg-muted/20">
      <AgentDetailsRows
        fields={fields}
        instruction={instruction}
        managedAgent={managedAgent}
        modelLabel={modelLabel}
        showInstructionPlaceholder={showInstructionPlaceholder}
        showModel={showModel}
      />
    </div>
  );
}

export function AgentDetailsRows({
  fields,
  instruction,
  managedAgent,
  modelLabel,
  showInstructionPlaceholder,
  showModel = false,
}: {
  fields: ProfileField[];
  instruction?: string | null;
  managedAgent?: ManagedAgent | undefined;
  modelLabel?: string;
  showInstructionPlaceholder?: boolean;
  showModel?: boolean;
}) {
  const trimmedInstruction = instruction?.trim() ?? "";
  const showInstructions =
    trimmedInstruction.length > 0 || showInstructionPlaceholder === true;
  const showModelRow =
    showModel === true &&
    (managedAgent !== undefined || (modelLabel?.trim().length ?? 0) > 0);

  if (!showInstructions && !showModelRow && fields.length === 0) {
    return null;
  }

  return (
    <>
      {showInstructions ? (
        <AgentInstructionRow instruction={instruction ?? null} />
      ) : null}

      {showModelRow ? (
        <AgentModelRow modelLabel={modelLabel ?? "Auto"} />
      ) : null}

      {fields.length > 0 ? <ProfileFieldRows fields={fields} /> : null}
    </>
  );
}

function hasAgentConfigurationRows({
  fields,
  instruction,
  managedAgent,
  modelLabel,
  showInstructionPlaceholder,
  showModel,
}: {
  fields: ProfileField[];
  instruction?: string | null;
  managedAgent: ManagedAgent | undefined;
  modelLabel: string;
  showInstructionPlaceholder?: boolean;
  showModel: boolean;
}) {
  const trimmedInstruction = instruction?.trim() ?? "";

  return (
    trimmedInstruction.length > 0 ||
    showInstructionPlaceholder === true ||
    (showModel === true &&
      (managedAgent !== undefined || modelLabel.trim().length > 0)) ||
    fields.length > 0
  );
}

export function AgentInstructionRow({
  instruction,
  onOpenInstructions,
}: {
  instruction: string | null;
  onOpenInstructions?: () => void;
}) {
  const trimmedInstruction = instruction?.trim() ?? "";
  const canOpenInstructions =
    trimmedInstruction.length > 0 && onOpenInstructions !== undefined;
  const rowContent = (
    <>
      <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-muted/60">
        <MessageSquare className="h-4 w-4 text-muted-foreground" />
      </span>
      <div className="min-w-0 flex-1 text-left">
        <div className="text-xs font-medium text-foreground">Instructions</div>
        {trimmedInstruction ? (
          canOpenInstructions ? (
            <span
              className="mt-1 line-clamp-2 whitespace-pre-wrap pr-1 text-sm leading-6"
              data-testid="user-profile-agent-instruction"
            >
              {trimmedInstruction}
            </span>
          ) : (
            <div
              className="mt-1 pr-1"
              data-testid="user-profile-agent-instruction"
            >
              <Markdown
                className="text-sm leading-6"
                content={trimmedInstruction}
                interactive={false}
              />
            </div>
          )
        ) : (
          <p
            className="mt-0.5 text-sm leading-6 text-muted-foreground"
            data-testid="user-profile-agent-instruction-empty"
          >
            No instruction set.
          </p>
        )}
      </div>
      {canOpenInstructions ? (
        <ChevronRight className="mt-2.5 h-4 w-4 shrink-0 text-muted-foreground" />
      ) : null}
    </>
  );

  if (canOpenInstructions) {
    return (
      <button
        className="flex w-full items-start gap-3 px-4 py-3 text-left transition-colors hover:bg-muted/40"
        data-testid="user-profile-agent-instruction-row"
        onClick={onOpenInstructions}
        type="button"
      >
        {rowContent}
      </button>
    );
  }

  return <div className="flex items-start gap-3 px-4 py-3">{rowContent}</div>;
}

export function AgentInstructionsFocusedView({
  instruction,
}: {
  instruction: string | null;
}) {
  const trimmedInstruction = instruction?.trim() ?? "";

  return (
    <div className="pt-4">
      <div
        className="rounded-2xl bg-muted/20 px-4 py-3"
        data-testid="user-profile-agent-instructions-view"
      >
        {trimmedInstruction ? (
          <Markdown
            className="text-sm leading-6"
            content={trimmedInstruction}
            interactive={false}
          />
        ) : (
          <p className="text-sm leading-6 text-muted-foreground">
            No instruction set.
          </p>
        )}
      </div>
    </div>
  );
}

function AgentModelRow({ modelLabel }: { modelLabel: string }) {
  return (
    <div
      className="flex items-center gap-3 px-4 py-3"
      data-testid="user-profile-model"
    >
      <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-muted/60">
        <Cpu className="h-4 w-4 text-muted-foreground" />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block text-xs font-medium text-foreground">Model</span>
        <span className="mt-0.5 block truncate text-sm text-muted-foreground">
          {modelLabel}
        </span>
      </span>
    </div>
  );
}
