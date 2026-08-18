import {
  ArrowRight,
  CalendarClock,
  CircleCheckBig,
  GitPullRequest,
  Hash,
  MessageCircle,
  MessageSquare,
  Send,
  SmilePlus,
  Timer,
  Webhook,
  Zap,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";

import type { Workflow } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { WorkflowActionsMenu } from "./WorkflowActionsMenu";
import {
  getWorkflowDescription,
  getWorkflowDisplayStatus,
  getWorkflowEnabled,
  getWorkflowPrimaryAction,
  getWorkflowTriggerSummary,
  getWorkflowTriggerType,
} from "./workflowDefinition";

type WorkflowCardProps = {
  workflow: Workflow;
  channelName?: string;
  isActive?: boolean;
  isTogglingEnabled?: boolean;
  onSelect: (workflowId: string) => void;
  onTrigger: (workflowId: string) => void;
  onToggleEnabled: (workflow: Workflow) => void;
  onEdit: (workflow: Workflow) => void;
  onDuplicate: (workflow: Workflow) => void;
  onDelete: (workflow: Workflow) => void;
};

const TRIGGER_ICONS: Record<string, LucideIcon> = {
  diff_posted: GitPullRequest,
  message_posted: MessageSquare,
  reaction_added: SmilePlus,
  schedule: CalendarClock,
  webhook: Webhook,
};

const ACTION_ICONS: Record<string, LucideIcon> = {
  add_reaction: SmilePlus,
  call_webhook: Webhook,
  delay: Timer,
  request_approval: CircleCheckBig,
  send_dm: MessageCircle,
  send_message: Send,
  set_channel_topic: Hash,
};

const TRIGGER_ACCENTS: Record<string, string> = {
  diff_posted: "border-violet-400/30 bg-violet-600 text-white",
  message_posted: "border-blue-400/30 bg-blue-600 text-white",
  reaction_added: "border-pink-400/30 bg-pink-600 text-white",
  schedule: "border-emerald-400/30 bg-emerald-600 text-white",
  webhook: "border-orange-300/30 bg-orange-500 text-white",
};

function StatusBadge({ status }: { status: Workflow["status"] }) {
  return (
    <span
      className={cn(
        "rounded-full border border-border/65 bg-background/80 px-2 py-1 text-2xs font-semibold uppercase tracking-wider shadow-xs",
        status === "active" ? "text-foreground" : "text-muted-foreground",
      )}
    >
      {status}
    </span>
  );
}

export function WorkflowCard({
  workflow,
  channelName,
  isActive = false,
  isTogglingEnabled = false,
  onSelect,
  onTrigger,
  onToggleEnabled,
  onEdit,
  onDuplicate,
  onDelete,
}: WorkflowCardProps) {
  const displayStatus = getWorkflowDisplayStatus(workflow);
  const triggerSummary = getWorkflowTriggerSummary(workflow.definition);
  const description = getWorkflowDescription(workflow.definition);
  const triggerType = getWorkflowTriggerType(workflow.definition);
  const actionType = getWorkflowPrimaryAction(workflow.definition);
  const TriggerIcon = triggerType ? TRIGGER_ICONS[triggerType] : undefined;
  const ActionIcon = actionType ? ACTION_ICONS[actionType] : undefined;
  const triggerAccent = triggerType ? TRIGGER_ACCENTS[triggerType] : undefined;

  return (
    <div
      className={cn(
        "group relative min-h-60 w-full overflow-hidden rounded-2xl border border-border/70 bg-muted/50 p-5 text-left text-foreground shadow-xs transition-all hover:-translate-y-0.5 hover:border-border hover:bg-muted/65 hover:shadow-md",
        isActive && "border-primary/50 bg-primary/5 ring-1 ring-primary/30",
      )}
      data-testid={`workflow-card-${workflow.id}`}
    >
      <button
        className="absolute inset-0 z-0 rounded-2xl focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
        onClick={() => onSelect(workflow.id)}
        type="button"
      >
        <span className="sr-only">View {workflow.name}</span>
      </button>

      <div className="pointer-events-none relative z-10 flex h-full min-h-48 flex-col">
        <div className="flex items-start justify-between gap-3">
          <div className="flex items-center gap-2" aria-hidden="true">
            <span
              className={cn(
                "flex h-9 w-9 items-center justify-center rounded-xl border shadow-xs",
                triggerAccent ?? "border-slate-400/30 bg-slate-600 text-white",
              )}
            >
              {TriggerIcon ? (
                <TriggerIcon className="h-5 w-5" />
              ) : (
                <Zap className="h-5 w-5" />
              )}
            </span>
            {ActionIcon ? (
              <>
                <ArrowRight className="h-4 w-4 text-muted-foreground/60" />
                <span className="flex h-9 w-9 items-center justify-center rounded-xl border border-border/65 bg-background/80 text-muted-foreground shadow-xs">
                  <ActionIcon className="h-5 w-5" />
                </span>
              </>
            ) : null}
          </div>

          <div className="pointer-events-auto flex items-center gap-1.5">
            <StatusBadge status={displayStatus} />
            <WorkflowActionsMenu
              isEnabled={getWorkflowEnabled(workflow.definition)}
              isTogglingEnabled={isTogglingEnabled}
              onDelete={() => onDelete(workflow)}
              onDuplicate={() => onDuplicate(workflow)}
              onEdit={() => onEdit(workflow)}
              onToggleEnabled={() => onToggleEnabled(workflow)}
              onTrigger={() => onTrigger(workflow.id)}
            />
          </div>
        </div>

        {triggerSummary ? (
          <p className="mt-4 line-clamp-1 text-xs font-semibold text-muted-foreground">
            {triggerSummary}
          </p>
        ) : null}
        <h3 className="mt-1 line-clamp-2 text-xl font-bold leading-tight tracking-tight">
          {workflow.name}
        </h3>
        {description ? (
          <p className="mt-2 line-clamp-2 text-sm leading-relaxed text-muted-foreground">
            {description}
          </p>
        ) : null}

        <div className="mt-auto flex min-w-0 items-end justify-between gap-3 pt-5 text-muted-foreground">
          <p className="min-w-0 truncate text-2xs">
            {channelName ? `#${channelName}` : "Channel workflow"}
          </p>
          <span className="shrink-0 text-2xs">
            {new Date(workflow.updatedAt * 1000).toLocaleDateString()}
          </span>
        </div>
      </div>
    </div>
  );
}
