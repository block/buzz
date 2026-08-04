import { cn } from "@/shared/lib/cn";
import { Switch } from "@/shared/ui/switch";

type AgentBehaviorSettingsProps = {
  agentRepliesInThreads: boolean;
  disabled: boolean;
  dmRequireMention: boolean;
  isDm: boolean;
  onAgentRepliesInThreadsChange: (checked: boolean) => void;
  onDmRequireMentionChange: (checked: boolean) => void;
};

function AgentBehaviorToggle({
  checked,
  description,
  disabled,
  label,
  onCheckedChange,
  testId,
}: {
  checked: boolean;
  description: string;
  disabled: boolean;
  label: string;
  onCheckedChange: (checked: boolean) => void;
  testId: string;
}) {
  return (
    <div
      className={cn(
        "flex items-start justify-between gap-4 rounded-2xl border border-border/60 px-4 py-3",
        disabled ? "cursor-not-allowed opacity-60" : "cursor-default",
      )}
    >
      <span className="min-w-0 flex-1">
        <span className="block text-sm font-medium text-foreground">
          {label}
        </span>
        <span className="mt-1 block text-xs leading-5 text-muted-foreground">
          {description}
        </span>
      </span>
      <Switch
        checked={checked}
        data-testid={testId}
        disabled={disabled}
        onCheckedChange={onCheckedChange}
      />
    </div>
  );
}

export function AgentBehaviorSettings({
  agentRepliesInThreads,
  disabled,
  dmRequireMention,
  isDm,
  onAgentRepliesInThreadsChange,
  onDmRequireMentionChange,
}: AgentBehaviorSettingsProps) {
  return (
    <div className="space-y-3" data-testid="channel-management-agent-behavior">
      <div>
        <h4 className="text-sm font-medium text-foreground">Agent behavior</h4>
        <p className="mt-1 text-xs leading-5 text-muted-foreground">
          Controls how agents in this channel wake up and place replies.
        </p>
      </div>
      <AgentBehaviorToggle
        checked={agentRepliesInThreads}
        description="Turn this off when agent responses should appear inline in the channel instead of opening a thread."
        disabled={disabled}
        label="Agents reply in threads"
        onCheckedChange={onAgentRepliesInThreadsChange}
        testId="channel-management-agent-replies-in-threads"
      />
      {isDm ? (
        <AgentBehaviorToggle
          checked={dmRequireMention}
          description="Turn this off so direct messages to the agent wake it without typing its name."
          disabled={disabled}
          label="Require @mention in this agent DM"
          onCheckedChange={onDmRequireMentionChange}
          testId="channel-management-dm-require-mention"
        />
      ) : null}
    </div>
  );
}
