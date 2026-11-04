// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { uiMessage, uiT } from "@/shared/i18n/uiTranslation";
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
      ? uiT("running")
      : uiT("stopped");

  return (
    <div className="space-y-2" data-testid="agent-instance-mode">
      <label className="text-sm font-medium" htmlFor="agent-instance-mode">
        {uiT("Agent instance")}
      </label>
      <select
        className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-2 text-sm shadow-xs"
        disabled={disabled}
        id="agent-instance-mode"
        onChange={(e) => onForceNewChange(e.target.value === "new")}
        value={forceNew ? "new" : "reuse"}
      >
        <option value="reuse">{uiT("Reuse existing agent")}</option>
        <option value="new">{uiT("Create new instance")}</option>
      </select>
      <p className="text-xs text-muted-foreground">
        {uiMessage(
          "{0} is already {1}. Reusing adds it to this channel without creating a duplicate keypair.",
          {
            p0: (
              <span className="font-medium text-foreground">
                {reusableAgent.name}
              </span>
            ),
            p1: statusLabel,
          },
        )}
      </p>
    </div>
  );
}
