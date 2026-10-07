import {
  ArrowRight,
  CalendarClock,
  CircleCheckBig,
  GitPullRequest,
  Hash,
  MessageCircle,
  MessageSquare,
  SmilePlus,
  Timer,
  Webhook,
  Zap,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { motion, useReducedMotion } from "motion/react";
import * as React from "react";

import type { Workflow } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Switch } from "@/shared/ui/switch";
import { WorkflowActionsMenu } from "./WorkflowActionsMenu";
import {
  getWorkflowEnabled,
  getWorkflowActionTiles,
  getWorkflowCardLabel,
  getWorkflowTriggerEmoji,
  getWorkflowTriggerType,
} from "./workflowDefinition";
import { StatusEmoji } from "@/features/user-status/ui/StatusEmoji";

type WorkflowCardProps = {
  workflow: Workflow;
  channelName?: string;
  isTogglingEnabled?: boolean;
  onView: (workflow: Workflow) => void;
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
  send_message: MessageSquare,
  set_channel_topic: Hash,
};

const TRIGGER_ACCENTS: Record<string, string> = {
  diff_posted: "border-violet-400/30 bg-violet-600 text-white",
  message_posted: "border-blue-400/30 bg-blue-600 text-white",
  reaction_added: "border-pink-400/30 bg-pink-600 text-white",
  schedule: "border-emerald-400/30 bg-emerald-600 text-white",
  webhook: "border-orange-300/30 bg-orange-500 text-white",
};

const ACTION_ACCENTS: Record<string, string> = {
  add_reaction: "border-pink-400/30 bg-pink-600 text-white",
  call_webhook: "border-orange-300/30 bg-orange-500 text-white",
  delay: "border-sky-300/30 bg-sky-500 text-white",
  request_approval: "border-emerald-300/30 bg-emerald-600 text-white",
  send_dm: "border-indigo-300/30 bg-indigo-600 text-white",
  send_message: "border-blue-300/30 bg-blue-600 text-white",
  set_channel_topic: "border-violet-300/30 bg-violet-600 text-white",
};

function StatusToggle({
  disabled,
  enabled,
  onToggle,
}: {
  disabled: boolean;
  enabled: boolean;
  onToggle: () => void;
}) {
  return (
    <Switch
      aria-label={enabled ? "Disable workflow" : "Enable workflow"}
      checked={enabled}
      disabled={disabled}
      onCheckedChange={(checked) => {
        if (checked !== enabled) onToggle();
      }}
    />
  );
}

function ActionTile({
  action,
  animationSequence,
  className,
  emoji,
  index,
}: {
  action: string;
  animationSequence: number;
  className?: string;
  emoji: string | null;
  index: number;
}) {
  const ActionIcon = ACTION_ICONS[action];
  const accent = ACTION_ACCENTS[action];
  const reduceMotion = useReducedMotion();

  return (
    <span
      aria-hidden="true"
      className={cn(
        "absolute inset-y-0 flex w-9 items-center justify-center rounded-xl border shadow-xs",
        accent ?? "border-border/65 bg-background/80 text-muted-foreground",
        className,
      )}
    >
      <motion.span
        animate={
          animationSequence > 0 && !reduceMotion
            ? { scale: [1, 1.18, 0.94, 1], y: [0, -5, 1, 0] }
            : undefined
        }
        className="flex items-center justify-center"
        transition={{
          delay: index * 0.11,
          duration: 0.48,
          ease: "easeOut",
        }}
      >
        {emoji ? (
          <StatusEmoji className="h-6 w-6 text-xl" value={emoji} />
        ) : ActionIcon ? (
          <ActionIcon className="h-5 w-5" />
        ) : (
          <Zap className="h-5 w-5" />
        )}
      </motion.span>
    </span>
  );
}

function ActionTileStack({
  actions,
  animationSequence = 0,
}: {
  actions: Array<{ action: string; emoji: string | null; key: string }>;
  animationSequence?: number;
}) {
  const visibleActions = actions.slice(0, 3);

  return (
    <span
      className={cn(
        "relative h-9",
        visibleActions.length === 1 && "w-9",
        visibleActions.length === 2 && "w-[2.625rem]",
        visibleActions.length > 2 && "w-12",
      )}
      data-testid="workflow-card-action-stack"
    >
      {visibleActions
        .map((action, index) => (
          <ActionTile
            action={action.action}
            animationSequence={animationSequence}
            className={cn(
              index === 0 && "left-0 z-10",
              index === 1 && "left-1.5 z-[5] scale-90 opacity-60",
              index === 2 && "left-3 scale-75 opacity-35",
            )}
            emoji={action.emoji}
            index={index}
            key={`${action.key}-${animationSequence}`}
          />
        ))
        .reverse()}
    </span>
  );
}

export function WorkflowCard({
  workflow,
  channelName,
  isTogglingEnabled = false,
  onView,
  onTrigger,
  onToggleEnabled,
  onEdit,
  onDuplicate,
  onDelete,
}: WorkflowCardProps) {
  const [triggerAnimationSequence, setTriggerAnimationSequence] =
    React.useState(0);
  const isEnabled = getWorkflowEnabled(workflow.definition);
  const cardLabel = getWorkflowCardLabel(workflow.definition);
  const triggerType = getWorkflowTriggerType(workflow.definition);
  const actionTiles = getWorkflowActionTiles(workflow.definition);
  const triggerEmoji = getWorkflowTriggerEmoji(workflow.definition);
  const TriggerIcon = triggerType ? TRIGGER_ICONS[triggerType] : undefined;
  const triggerAccent = triggerType ? TRIGGER_ACCENTS[triggerType] : undefined;

  return (
    <div
      className={cn(
        "group relative flex min-h-60 w-full flex-col overflow-hidden rounded-2xl bg-muted/50 p-5 text-left text-foreground shadow-xs transition-colors hover:bg-muted/65",
      )}
      data-testid={`workflow-card-${workflow.id}`}
    >
      <button
        className="absolute inset-0 z-0 rounded-2xl focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
        onClick={() => onView(workflow)}
        type="button"
      >
        <span className="sr-only">View {workflow.name}</span>
      </button>

      <div className="pointer-events-none relative z-10 flex min-h-48 flex-1 flex-col">
        <div className="flex items-start justify-between gap-3">
          <div className="flex items-center gap-2" aria-hidden="true">
            <span
              className={cn(
                "flex h-9 w-9 items-center justify-center rounded-xl border shadow-xs",
                triggerAccent ?? "border-slate-400/30 bg-slate-600 text-white",
              )}
            >
              {triggerEmoji ? (
                <StatusEmoji className="h-6 w-6 text-xl" value={triggerEmoji} />
              ) : TriggerIcon ? (
                <TriggerIcon className="h-5 w-5" />
              ) : (
                <Zap className="h-5 w-5" />
              )}
            </span>
            {actionTiles.length > 0 ? (
              <>
                <ArrowRight className="h-4 w-4 text-muted-foreground/60" />
                <ActionTileStack
                  actions={actionTiles}
                  animationSequence={triggerAnimationSequence}
                />
              </>
            ) : null}
          </div>

          <div className="pointer-events-auto flex items-center gap-1.5">
            <StatusToggle
              disabled={isTogglingEnabled}
              enabled={isEnabled}
              onToggle={() => onToggleEnabled(workflow)}
            />
            <WorkflowActionsMenu
              isEnabled={isEnabled}
              isTogglingEnabled={isTogglingEnabled}
              onDelete={() => onDelete(workflow)}
              onDuplicate={() => onDuplicate(workflow)}
              onEdit={() => onEdit(workflow)}
              onToggleEnabled={() => onToggleEnabled(workflow)}
              onTrigger={() => {
                setTriggerAnimationSequence((sequence) => sequence + 1);
                onTrigger(workflow.id);
              }}
              showEnabledToggle={false}
            />
          </div>
        </div>

        <h3
          className="mt-4 line-clamp-4 text-xl font-bold leading-tight tracking-tight"
          data-testid="workflow-card-semantic-label"
        >
          {cardLabel}
        </h3>

        <div className="mt-auto flex min-w-0 items-end justify-between gap-3 pt-5 text-muted-foreground">
          <div className="min-w-0">
            {channelName ? (
              <p
                className="truncate text-xs font-semibold text-foreground"
                data-testid="workflow-card-channel"
              >
                #{channelName}
              </p>
            ) : null}
            <p
              className={cn(
                "truncate text-2xs text-muted-foreground",
                channelName && "mt-0.5",
              )}
              data-testid="workflow-card-name"
            >
              {workflow.name}
            </p>
          </div>
          <span className="shrink-0 text-2xs">
            {new Date(workflow.updatedAt * 1000).toLocaleDateString()}
          </span>
        </div>
      </div>
    </div>
  );
}
