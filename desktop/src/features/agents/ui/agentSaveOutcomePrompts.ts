import { toast } from "sonner";

import type { useStartManagedAgentMutation } from "@/features/agents/hooks";
import {
  respawnManagedAgentWithRules,
  shouldOfferImmediateRespondToRestart,
} from "@/features/agents/lib/managedAgentControlActions";
import { clearActiveTurnsForAgentOnStop } from "@/features/agents/managedAgentRuntimeHooks";
import {
  startManagedAgent,
  stopManagedAgent,
} from "@/shared/api/tauriManagedAgents";
import type { ManagedAgent, UpdateManagedAgentInput } from "@/shared/api/types";

/**
 * After a managed-agent edit saves, offer an immediate restart if the save
 * changed "Who can talk to this agent" on a still-running instance.
 *
 * `respond_to`/`respond_to_allowlist` is a security-relevant gate: the
 * running process keeps its OLD env until restarted (`update_managed_agent`
 * never auto-restarts), and the passive "Restart required" badge is easy to
 * miss for a setting whose whole point is to take effect promptly. This
 * mirrors the existing "saved while stopped → Start now" toast pattern in
 * `AgentInstanceEditDialog`, but for the running/needs-restart case (buzz#2501,
 * buzz#2950: agents set to `anyone`/`allowlist` stayed unreachable to
 * non-owners because the running harness never picked up the change).
 */
export function offerRespondToRestartIfNeeded(
  input: Pick<UpdateManagedAgentInput, "respondTo" | "respondToAllowlist">,
  agent: ManagedAgent,
) {
  if (!shouldOfferImmediateRespondToRestart(input, agent.needsRestart)) {
    return;
  }

  const restartedName = agent.name;
  toast(`${restartedName} saved — restart to apply the new setting.`, {
    action: {
      label: "Restart now",
      onClick: () => {
        respawnManagedAgentWithRules({
          agent,
          startManagedAgent,
          stopManagedAgent,
          onStopped: () => clearActiveTurnsForAgentOnStop(agent.pubkey),
        })
          .then(() => toast.success(`${restartedName} restarted.`))
          .catch((error: unknown) =>
            toast.error(
              error instanceof Error
                ? `${restartedName} failed to restart: ${error.message}`
                : `${restartedName} failed to restart.`,
            ),
          );
      },
    },
  });
}

/**
 * The auto-restart policy deliberately never fires for a stopped or failing
 * agent (a broken agent must not auto-loop), so an edit meant to FIX one
 * silently waits for a manual start. Offer that start explicitly instead of
 * relying on the user to know the policy.
 */
export function offerStartNowPrompt(
  agent: ManagedAgent,
  startMutation: ReturnType<typeof useStartManagedAgentMutation>,
) {
  const startedName = agent.name;
  toast(`${startedName} saved while stopped.`, {
    action: {
      label: "Start now",
      onClick: () => {
        startMutation.mutate(agent.pubkey, {
          onSuccess: () => toast.success(`${startedName} started.`),
          onError: (error) =>
            toast.error(
              error instanceof Error
                ? `${startedName} failed to start: ${error.message}`
                : `${startedName} failed to start.`,
            ),
        });
      },
    },
  });
}
