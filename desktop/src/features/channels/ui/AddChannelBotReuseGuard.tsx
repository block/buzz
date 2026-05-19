import type { ManagedAgent } from "@/shared/api/types";

type AddChannelBotReuseGuardProps = {
  reusableAgent: ManagedAgent;
  forceNew: boolean;
  onForceNewChange: (forceNew: boolean) => void;
  disabled: boolean;
};

export function AddChannelBotReuseGuard({
  reusableAgent,
  forceNew,
  onForceNewChange,
  disabled,
}: AddChannelBotReuseGuardProps) {
  const statusLabel =
    reusableAgent.status === "running" || reusableAgent.status === "deployed"
      ? "running"
      : "stopped";

  return (
    <div className="space-y-2 rounded-2xl border border-border/70 bg-card/70 p-4">
      <div className="text-sm font-medium">Existing agent found</div>
      <p className="text-xs text-muted-foreground">
        <span className="font-medium text-foreground">
          {reusableAgent.name}
        </span>{" "}
        is already {statusLabel}. You can reuse it in this channel or create a
        separate instance with its own identity.
      </p>
      <div className="mt-3 space-y-2">
        <label
          className={`flex cursor-pointer items-center gap-2 rounded-lg border px-3 py-2 text-sm transition-colors ${
            !forceNew
              ? "border-primary/40 bg-primary/5"
              : "border-border/50 bg-transparent"
          } ${disabled ? "pointer-events-none opacity-50" : ""}`}
        >
          <input
            checked={!forceNew}
            className="accent-primary"
            disabled={disabled}
            name="agent-reuse-choice"
            onChange={() => onForceNewChange(false)}
            type="radio"
          />
          <span>Add to this channel (reuse existing agent)</span>
        </label>
        <label
          className={`flex cursor-pointer items-center gap-2 rounded-lg border px-3 py-2 text-sm transition-colors ${
            forceNew
              ? "border-primary/40 bg-primary/5"
              : "border-border/50 bg-transparent"
          } ${disabled ? "pointer-events-none opacity-50" : ""}`}
        >
          <input
            checked={forceNew}
            className="accent-primary"
            disabled={disabled}
            name="agent-reuse-choice"
            onChange={() => onForceNewChange(true)}
            type="radio"
          />
          <span>Create new instance (separate identity &amp; keypair)</span>
        </label>
      </div>
    </div>
  );
}
