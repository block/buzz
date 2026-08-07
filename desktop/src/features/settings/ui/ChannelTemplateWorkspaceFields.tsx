import { ChevronRight, GitBranch, MessageSquare } from "lucide-react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";

import {
  CHANNEL_FORM_FIELD_CONTROL_CLASS,
  CHANNEL_FORM_FIELD_SHELL_CLASS,
} from "@/features/channels/ui/channelFormStyles";
import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";
import { Switch } from "@/shared/ui/switch";
import { Textarea } from "@/shared/ui/textarea";

export const DEFAULT_WORKTREE_LOCATION = "~/.buzz/worktrees";
export const DEFAULT_WORKTREE_BASE_BRANCH = "main";

export function ChannelTemplateWorktreeFields({
  baseBranch,
  disabled,
  enabled,
  location,
  onBaseBranchChange,
  onEnabledChange,
  onLocationChange,
  projectFolders,
}: {
  baseBranch: string;
  disabled: boolean;
  enabled: boolean;
  location: string;
  onBaseBranchChange: (value: string) => void;
  onEnabledChange: (enabled: boolean) => void;
  onLocationChange: (value: string) => void;
  projectFolders: readonly string[];
}) {
  return (
    <div
      className="shrink-0 overflow-hidden rounded-xl border border-input bg-background"
      data-testid="channel-template-worktree-section"
    >
      <div
        className="flex min-h-11 items-center gap-3 px-3"
        data-testid="channel-template-worktree-toggle-row"
      >
        <GitBranch
          aria-hidden
          className="h-4 w-4 shrink-0 text-muted-foreground"
        />
        <label
          className="min-w-0 flex-1 text-sm font-medium text-foreground"
          htmlFor="channel-template-worktree-toggle"
        >
          Work in a new worktree
        </label>
        <Switch
          checked={enabled}
          data-testid="channel-template-worktree-toggle"
          disabled={disabled}
          id="channel-template-worktree-toggle"
          onCheckedChange={onEnabledChange}
        />
      </div>

      {enabled ? (
        <div
          className="space-y-3 border-t border-border/60 p-3"
          data-testid="channel-template-worktree-fields"
        >
          <div className="grid grid-cols-2 gap-3">
            <div className="space-y-1.5">
              <label
                className="text-xs font-medium text-foreground"
                htmlFor="template-worktree-location"
              >
                Worktree location
              </label>
              <div
                className={cn(
                  "flex min-h-10 items-center px-3",
                  CHANNEL_FORM_FIELD_SHELL_CLASS,
                )}
              >
                <Input
                  autoCapitalize="none"
                  autoComplete="off"
                  autoCorrect="off"
                  className={cn(
                    "h-8 px-0 py-0 leading-6",
                    CHANNEL_FORM_FIELD_CONTROL_CLASS,
                  )}
                  disabled={disabled}
                  id="template-worktree-location"
                  onChange={(event) => onLocationChange(event.target.value)}
                  placeholder={`Default: ${DEFAULT_WORKTREE_LOCATION}`}
                  spellCheck={false}
                  value={location}
                />
              </div>
            </div>

            <div className="space-y-1.5">
              <label
                className="text-xs font-medium text-foreground"
                htmlFor="template-worktree-base-branch"
              >
                Base branch
              </label>
              <div
                className={cn(
                  "flex min-h-10 items-center px-3",
                  CHANNEL_FORM_FIELD_SHELL_CLASS,
                )}
              >
                <Input
                  autoCapitalize="none"
                  autoComplete="off"
                  autoCorrect="off"
                  className={cn(
                    "h-8 px-0 py-0 leading-6",
                    CHANNEL_FORM_FIELD_CONTROL_CLASS,
                  )}
                  disabled={disabled}
                  id="template-worktree-base-branch"
                  onChange={(event) => onBaseBranchChange(event.target.value)}
                  placeholder={DEFAULT_WORKTREE_BASE_BRANCH}
                  spellCheck={false}
                  value={baseBranch}
                />
              </div>
            </div>
          </div>

          <p
            className="rounded-lg border border-border/60 bg-muted/20 px-3 py-2 text-xs text-muted-foreground"
            data-testid="channel-template-worktree-guidance"
          >
            {projectFolders.length === 1
              ? `With worktrees enabled, use ${projectFolders[0]} for review only. New changes belong in the worktree.`
              : projectFolders.length > 1
                ? `With worktrees enabled, use these ${projectFolders.length} project folders for review only. New changes belong in worktrees.`
                : "Choose a project folder above. With worktrees enabled, it is for review only and new changes belong in the worktree."}
          </p>
        </div>
      ) : null}
    </div>
  );
}

export function ChannelTemplateCanvasSection({
  disabled,
  onChange,
  onOpenChange,
  open,
  value,
}: {
  disabled: boolean;
  onChange: (value: string) => void;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  value: string;
}) {
  const shouldReduceMotion = useReducedMotion() ?? false;

  return (
    <div className="shrink-0 overflow-hidden rounded-xl border border-input bg-background">
      <button
        aria-controls="channel-template-canvas-panel"
        aria-expanded={open}
        className="flex w-full items-center gap-3 px-3 py-2.5 text-left text-sm transition-colors hover:bg-muted/40 focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-ring"
        data-testid="channel-template-canvas-toggle"
        disabled={disabled}
        onClick={() => onOpenChange(!open)}
        type="button"
      >
        <MessageSquare
          aria-hidden
          className="h-4 w-4 shrink-0 text-muted-foreground"
        />
        <span className="min-w-0 flex-1">
          <span className="flex items-center gap-2">
            <span className="font-medium text-foreground">Canvas</span>
            <span className="text-xs text-muted-foreground/55">Optional</span>
          </span>
          <span className="mt-0.5 block text-xs leading-4 text-muted-foreground">
            Add instructions and context that agents can consult while working
            in a channel created from this template.
          </span>
        </span>
        <ChevronRight
          aria-hidden
          className={cn(
            "ml-auto h-4 w-4 shrink-0 text-muted-foreground transition-transform",
            open && "rotate-90",
          )}
        />
      </button>
      <AnimatePresence initial={false}>
        {open ? (
          <motion.div
            animate={{ height: "auto", opacity: 1 }}
            data-testid="channel-template-canvas-panel"
            exit={{ height: 0, opacity: shouldReduceMotion ? 1 : 0 }}
            initial={shouldReduceMotion ? false : { height: 0, opacity: 0 }}
            style={{ overflow: "hidden" }}
            transition={{
              duration: shouldReduceMotion ? 0 : 0.22,
              ease: [0.77, 0, 0.175, 1],
            }}
          >
            <div
              className="space-y-2 border-t border-border/60 p-3"
              id="channel-template-canvas-panel"
            >
              <Textarea
                className={cn(
                  "min-h-24 resize-none px-3 py-3 font-mono text-xs leading-5",
                  CHANNEL_FORM_FIELD_CONTROL_CLASS,
                )}
                data-testid="channel-template-canvas"
                disabled={disabled}
                id="template-canvas"
                onChange={(event) => onChange(event.target.value)}
                placeholder="Canvas content here..."
                rows={4}
                value={value}
              />
              <p className="text-xs text-muted-foreground">
                Use {"{channel.name}"} and {"{template.name}"} as placeholders.
              </p>
            </div>
          </motion.div>
        ) : null}
      </AnimatePresence>
    </div>
  );
}
