import { Switch } from "@/shared/ui/switch";

/** Env var buzz-acp reads at spawn: "ask" routes tool permission requests to you (desktop or web over Direct). */
export const PERMISSION_MODE_ENV = "BUZZ_ACP_PERMISSION_MODE";
/** Explicit "don't ask" value, used to override an agent definition that asks. Parses as buzz-acp's default mode. */
export const PERMISSION_MODE_NO_ASK = "bypassPermissions";

export function envAsks(env: Record<string, string> | null | undefined): boolean {
  return String(env?.[PERMISSION_MODE_ENV] ?? "").trim() === "ask";
}

/**
 * Returns `env` with permission-ask turned on/off. Turning it off drops the key, unless `inherited` (the agent
 * definition's env) asks, in which case the agent overrides it with an explicit no-ask value.
 */
export function withPermissionAsk(
  env: Record<string, string>,
  ask: boolean,
  inherited?: Record<string, string> | null,
): Record<string, string> {
  const next = { ...env };
  if (ask) next[PERMISSION_MODE_ENV] = "ask";
  else if (envAsks(inherited)) next[PERMISSION_MODE_ENV] = PERMISSION_MODE_NO_ASK;
  else delete next[PERMISSION_MODE_ENV];
  return next;
}

/** "Ask before using tools" switch for the create-agent and edit-agent forms. */
export function PermissionAskToggleField({
  checked,
  disabled = false,
  disabledReason,
  id,
  note,
  onCheckedChange,
}: {
  checked: boolean;
  disabled?: boolean;
  disabledReason?: string;
  id: string;
  note?: string;
  onCheckedChange: (value: boolean) => void;
}) {
  return (
    <section
      className="space-y-2 rounded-lg border border-border/60 bg-muted/20 p-3"
      data-testid={id}
    >
      <div className="flex items-center justify-between gap-3">
        <div className="min-w-0 space-y-0.5">
          <label className="text-sm font-semibold text-foreground" htmlFor={id}>
            Ask before using tools
          </label>
          <p className="text-xs text-muted-foreground">
            {disabledReason ??
              (checked
                ? "The agent asks you before running tools. Approve from desktop or web."
                : "The agent uses tools without asking.")}
            {note ? ` ${note}` : ""}
          </p>
        </div>
        {/* Larger hit area on touch screens (phones) — the bare switch is a small target. */}
        <span className="-m-2 inline-flex p-2">
          <Switch
            aria-label="Ask before using tools"
            checked={checked}
            disabled={disabled}
            id={id}
            onCheckedChange={onCheckedChange}
          />
        </span>
      </div>
    </section>
  );
}
