/**
 * AgentEditDialog.tsx — Merged agent edit surface entry point (Phase 1 Round 2).
 *
 * Collapses R1–R7 onto a single dialog: AgentEditMergedDialog.
 *
 * All contexts — instance-with-definition, instance-only, definition-only —
 * render the ONE merged surface. AgentEditMergedDialog seeds from
 * AgentFormModel, renders D+I+L sections per context, and submits every
 * save through the Artifact 3 coordinator (observed-state settlement for D,
 * I, and per-policy writes).
 *
 * AgentDefinitionDialog survives for create flows (R8 duplicate seed, import)
 * only — it is NOT a consumer of any R1–R7 edit route.
 * AgentInstanceEditDialog has no production consumer on any R1–R7 route.
 */

import type { ManagedAgent } from "@/shared/api/types";
import type { EditAgentFocusTarget } from "@/features/agents/openEditAgentEvent";
import type { AgentEditContext } from "./agentFormModel";
export type { AgentEditContext };
import { runLocationForBackend } from "@/features/agents/lib/agentAccessWarning";
import { AgentRunLocationProvider } from "./AgentRunLocationContext";
import { AgentEditMergedDialog } from "./AgentEditMergedDialog";

// ── Types ─────────────────────────────────────────────────────────────────────

export type AgentEditDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Edit context: which entity or entities to edit. */
  ctx: AgentEditContext;
  /**
   * Optional field to focus when the dialog opens from a card deep-link.
   */
  initialFocus?: EditAgentFocusTarget;
  onUpdated?: (agent: ManagedAgent) => void;
  /**
   * Optional pre-save validator (R6 origin permission check).
   * Return a non-null string to abort with an error toast; return null to proceed.
   */
  onValidate?: () => string | null;
  /**
   * Optional initial-values override for definition fields (R6 review mode).
   */
  initialValueOverrides?: Partial<{
    displayName: string;
    systemPrompt: string;
    runtime: string | undefined;
    provider: string | undefined;
    model: string | undefined;
    respondTo: string | undefined;
  }>;
  /**
   * Definition admin actions forwarded to AgentEditMergedDialog (Artifact 4).
   * Omit when the host does not own those actions.
   */
  onDeleteDefinition?: () => void;
  onRemoveFromMyAgents?: () => void;
  onShare?: () => void;
  linkedInstanceCount?: number;
  isIdentityArchived?: boolean;
};

// ── Component ─────────────────────────────────────────────────────────────────

export function AgentEditDialog({
  ctx,
  open,
  onOpenChange,
  onUpdated,
  onValidate,
  initialValueOverrides,
  initialFocus,
  onDeleteDefinition,
  onRemoveFromMyAgents,
  onShare,
  linkedInstanceCount,
  isIdentityArchived,
}: AgentEditDialogProps) {
  // All contexts route to the merged surface. The run-location provider wraps
  // instance-present paths so RespondToField's remote-backend warning can name
  // the machine the agent actually runs on.
  const inst = ctx.kind !== "definition-only" ? ctx.instance : null;
  const runLocation = inst ? runLocationForBackend(inst.backend) : null;

  const inner = (
    <AgentEditMergedDialog
      ctx={ctx}
      initialFocus={initialFocus}
      initialValueOverrides={initialValueOverrides}
      isIdentityArchived={isIdentityArchived}
      linkedInstanceCount={linkedInstanceCount}
      onDeleteDefinition={onDeleteDefinition}
      onOpenChange={onOpenChange}
      onRemoveFromMyAgents={onRemoveFromMyAgents}
      onShare={onShare}
      onUpdated={onUpdated}
      onValidate={onValidate}
      open={open}
    />
  );

  if (inst) {
    return (
      <AgentRunLocationProvider runLocation={runLocation}>
        {inner}
      </AgentRunLocationProvider>
    );
  }

  return inner;
}
