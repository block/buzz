import type { ManagedAgent } from "@/shared/api/types";

const SOURCE_LABEL: Record<string, string> = {
  agent: "agent override",
  definition: "agent definition",
  global_default: "global default",
  built_in: "built-in",
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
};

/**
 * Instance-side, display-only view of an agent's effective permission policy.
 *
 * The editable default lives on the agent definition (create / edit-agent);
 * this surface never sets policy. It shows the resolved effective value and,
 * for a remotely deployed agent, the applied-vs-desired drift row that a
 * per-instance receipt makes possible. A definition has no deploy receipt, so
 * drift is inherently instance-scoped and stays here.
 */
export function AgentPermissionPolicyField({ agent }: Props) {
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
        <span className="text-sm font-medium text-foreground">
          Permission policy
        </span>
        <span className="text-xs text-muted-foreground">
          ({agent.permissionPolicy} · from {sourceLabel})
        </span>
      </div>
      <p className="text-xs text-muted-foreground">
        {isRemoteDeployed
          ? "Read-only while deployed. To change, edit the agent definition and redeploy."
          : "Set the default in the agent definition (Create / Edit agent)."}
      </p>
      {hasDrift && (
        <p className="text-xs text-amber-600 dark:text-amber-400">
          Applied policy:{" "}
          <span className="font-medium">{agent.appliedPermissionPolicy}</span> ·
          Desired: <span className="font-medium">{agent.permissionPolicy}</span>{" "}
          — redeploy required to apply.
        </p>
      )}
    </div>
  );
}
