import type { GlobalAgentConfig } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { AgentDropdownSelect } from "./agentConfigControls";
import {
  applyGuardianPolicy,
  GUARDIAN_POLICY_ENV,
  GUARDIAN_POLICY_OPTIONS,
} from "./guardianPolicy";

export function GuardianPolicyField({
  blockClassName,
  config,
  fieldLabelClassName,
  onConfigChange,
}: {
  blockClassName?: string;
  config: GlobalAgentConfig;
  fieldLabelClassName?: string;
  onConfigChange: (config: GlobalAgentConfig) => void;
}) {
  return (
    <div className={blockClassName}>
      <label
        className={cn("text-sm font-medium", fieldLabelClassName)}
        htmlFor="global-agent-guardian-policy"
      >
        Tool permission policy
      </label>
      <AgentDropdownSelect
        ariaDescribedBy="global-agent-guardian-policy-description"
        id="global-agent-guardian-policy"
        onValueChange={(value) =>
          onConfigChange(applyGuardianPolicy(config, value))
        }
        options={[...GUARDIAN_POLICY_OPTIONS]}
        testId="global-agent-guardian-policy"
        value={config.env_vars[GUARDIAN_POLICY_ENV] ?? "default"}
      />
      <p
        className="mt-1 text-xs text-muted-foreground"
        id="global-agent-guardian-policy-description"
      >
        Monitor allows permission requests and records each decision. Lockdown
        denies permission requests before the tool runs.
      </p>
    </div>
  );
}
