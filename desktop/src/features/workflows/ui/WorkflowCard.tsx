import {
  Clock,
  Copy,
  MoreHorizontal,
  Pencil,
  Play,
  Trash2,
  Zap,
} from "lucide-react";

import type { Workflow } from "@/shared/api/types";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import {
  getWorkflowDescription,
  getWorkflowDisplayStatus,
  getWorkflowTriggerSummary,
} from "./workflowDefinition";

type WorkflowCardProps = {
  workflow: Workflow;
  channelName?: string;
  isActive?: boolean;
  onSelect: (workflowId: string) => void;
  onTrigger: (workflowId: string) => void;
  onEdit: (workflow: Workflow) => void;
  onDuplicate: (workflow: Workflow) => void;
  onDelete: (workflow: Workflow) => void;
};

function StatusBadge({ status }: { status: Workflow["status"] }) {
  const variants: Record<
    Workflow["status"],
    "success" | "secondary" | "warning"
  > = {
    active: "success",
    disabled: "secondary",
    archived: "warning",
  };

  return <Badge variant={variants[status]}>{status}</Badge>;
}

export function WorkflowCard({
  workflow,
  channelName,
  isActive = false,
  onSelect,
  onTrigger,
  onEdit,
  onDuplicate,
  onDelete,
}: WorkflowCardProps) {
  const displayStatus = getWorkflowDisplayStatus(workflow);
  const description = getWorkflowDescription(workflow.definition);
  const triggerSummary = getWorkflowTriggerSummary(workflow.definition);

  return (
    <div
      className={`relative w-full rounded-lg border bg-card p-3 text-left transition-colors hover:bg-muted/50 ${
        isActive ? "border-primary/40 bg-primary/5 shadow-xs" : ""
      }`}
      data-testid={`workflow-card-${workflow.id}`}
    >
      <button
        className="absolute inset-0 rounded-lg"
        onClick={() => onSelect(workflow.id)}
        type="button"
      >
        <span className="sr-only">View {workflow.name}</span>
      </button>

      <div className="flex items-start justify-between">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <Zap className="h-4 w-4 shrink-0 text-amber-500" />
            <span className="truncate text-sm font-medium">
              {workflow.name}
            </span>
            <StatusBadge status={displayStatus} />
          </div>
          <div className="mt-1.5 flex items-center gap-3 pl-6 text-[11px] text-muted-foreground">
            {channelName ? <span>{channelName}</span> : null}
            {triggerSummary ? <span>{triggerSummary}</span> : null}
            <span className="flex items-center gap-1">
              <Clock className="h-4 w-4" />
              {new Date(workflow.updatedAt * 1000).toLocaleDateString()}
            </span>
          </div>
          {description ? (
            <p className="mt-2 pl-6 text-xs text-muted-foreground">
              {description}
            </p>
          ) : null}
        </div>

        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              aria-label="Workflow actions"
              className="relative z-10 h-7 w-7 shrink-0"
              size="icon"
              variant="ghost"
            >
              <MoreHorizontal className="h-4 w-4" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onClick={() => onTrigger(workflow.id)}>
              <Play className="mr-2 h-4 w-4" />
              Trigger
            </DropdownMenuItem>
            <DropdownMenuItem onClick={() => onEdit(workflow)}>
              <Pencil className="mr-2 h-4 w-4" />
              Edit
            </DropdownMenuItem>
            <DropdownMenuItem onClick={() => onDuplicate(workflow)}>
              <Copy className="mr-2 h-4 w-4" />
              Duplicate
            </DropdownMenuItem>
            <DropdownMenuItem
              className="text-destructive"
              onClick={() => onDelete(workflow)}
            >
              <Trash2 className="mr-2 h-4 w-4" />
              Delete
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </div>
  );
}
