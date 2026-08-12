import React from "react";
import type { ManagedAgent, PermissionPolicy } from "@/shared/api/types";

const SOURCE_LABEL: Record<string, string> = {
  agent: "agent override",
  global_default: "global default",
  built_in: "built-in",
};

function initialValue(
  agent: Pick<ManagedAgent, "permissionPolicy" | "permissionPolicySource">,
): PermissionPolicy | null {
  return agent.permissionPolicySource === "agent"
    ? agent.permissionPolicy
    : null;
}

export type AgentPermissionPolicyFieldHandle = {
  reset(): void;
  getUpdate(): PermissionPolicy | null | undefined;
};

type Props = {
  agent: Pick<
    ManagedAgent,
    | "backend"
    | "backendAgentId"
    | "permissionPolicy"
    | "permissionPolicySource"
    | "appliedPermissionPolicy"
  >;
  disabled: boolean;
};

/** Self-managing permission policy selector. Expose reset/getUpdate via ref. */
export const AgentPermissionPolicyField = React.forwardRef<
  AgentPermissionPolicyFieldHandle,
  Props
>(function AgentPermissionPolicyField({ agent, disabled }, ref) {
  const [value, setValue] = React.useState<PermissionPolicy | null>(() =>
    initialValue(agent),
  );

  React.useImperativeHandle(ref, () => ({
    reset: () => setValue(initialValue(agent)),
    getUpdate: () => (value !== initialValue(agent) ? value : undefined),
  }));

  const isRemoteDeployed =
    agent.backend.type === "provider" && agent.backendAgentId !== null;
  const sourceLabel =
    SOURCE_LABEL[agent.permissionPolicySource] ?? agent.permissionPolicySource;

  const hasDrift =
    isRemoteDeployed &&
    agent.appliedPermissionPolicy !== null &&
    agent.appliedPermissionPolicy !== agent.permissionPolicy;

  return (
    <div className="space-y-1.5">
      <div className="flex items-center gap-1.5">
        <label
          className="text-sm font-medium text-foreground"
          htmlFor="edit-agent-permission-policy"
        >
          Permission policy
        </label>
        <span className="text-xs text-muted-foreground">
          ({agent.permissionPolicy} · from {sourceLabel})
        </span>
      </div>
      {isRemoteDeployed ? (
        <>
          <p className="text-xs text-muted-foreground">
            Read-only while deployed. To change, shut down and redeploy the
            agent.
          </p>
          {hasDrift && (
            <p className="text-xs text-amber-600 dark:text-amber-400">
              Applied policy:{" "}
              <span className="font-medium">
                {agent.appliedPermissionPolicy}
              </span>{" "}
              · Desired:{" "}
              <span className="font-medium">{agent.permissionPolicy}</span> —
              redeploy required to apply.
            </p>
          )}
        </>
      ) : (
        <select
          className="w-full rounded-md border border-input bg-background px-3 py-1.5 text-sm shadow-sm focus:outline-none focus:ring-1 focus:ring-ring disabled:opacity-50"
          disabled={disabled}
          id="edit-agent-permission-policy"
          value={value ?? ""}
          onChange={(e) => {
            const val = e.target.value;
            setValue(val === "" ? null : (val as PermissionPolicy));
          }}
        >
          <option value="">
            Inherit ({agent.permissionPolicy} · from {sourceLabel})
          </option>
          <option value="ask">Ask — show Allow/Deny card</option>
          <option value="allow">Allow — auto-approve (explicit opt-in)</option>
          <option value="reject">Reject — auto-deny</option>
        </select>
      )}
    </div>
  );
});
