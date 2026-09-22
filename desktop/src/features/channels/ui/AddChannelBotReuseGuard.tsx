import type { ManagedAgent } from "@/shared/api/types";
import { useTranslation } from "@/i18n";

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
  const { t } = useTranslation();
  const statusLabel =
    reusableAgent.status === "running" || reusableAgent.status === "deployed"
      ? t("channels.bot.status-running")
      : t("channels.bot.status-stopped");

  return (
    <div className="space-y-2" data-testid="agent-instance-mode">
      <label className="text-sm font-medium" htmlFor="agent-instance-mode">
        {t("channels.bot.instance-label")}
      </label>
      <select
        className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-2 text-sm shadow-xs"
        disabled={disabled}
        id="agent-instance-mode"
        onChange={(e) => onForceNewChange(e.target.value === "new")}
        value={forceNew ? "new" : "reuse"}
      >
        <option value="reuse">{t("channels.bot.option-reuse")}</option>
        <option value="new">{t("channels.bot.option-new")}</option>
      </select>
      <p className="text-xs text-muted-foreground">
        {t("channels.bot.reuse-note", {
          name: reusableAgent.name,
          status: statusLabel,
        })}
      </p>
    </div>
  );
}
