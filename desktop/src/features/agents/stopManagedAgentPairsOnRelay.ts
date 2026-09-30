import { canonicalRelayUrl } from "@/features/agents/managedAgentRuntimeStatus";
import {
  listManagedAgentRuntimes,
  stopManagedAgentRuntime,
} from "@/shared/api/tauriManagedAgents";
import type { ManagedAgentRuntimeStatus } from "@/shared/api/types";

type Dependencies = {
  list: () => Promise<ManagedAgentRuntimeStatus[]>;
  stop: (pubkey: string, relayUrl: string) => Promise<unknown>;
};

const defaultDependencies: Dependencies = {
  list: listManagedAgentRuntimes,
  stop: stopManagedAgentRuntime,
};

/**
 * Stop every live agent pair on a relay whose community was removed.
 * Best-effort: failures are logged so community removal still completes.
 */
export async function stopManagedAgentPairsOnRelay(
  relayUrl: string,
  dependencies: Dependencies = defaultDependencies,
): Promise<void> {
  const relay = canonicalRelayUrl(relayUrl);
  if (!relay) return;
  try {
    const pairs = (await dependencies.list()).filter(
      (runtime) =>
        runtime.lifecycle !== "stopped" &&
        canonicalRelayUrl(runtime.relayUrl) === relay,
    );
    const results = await Promise.allSettled(
      pairs.map((pair) => dependencies.stop(pair.pubkey, pair.relayUrl)),
    );
    for (const result of results) {
      if (result.status === "rejected") {
        console.warn("[managed-agent-runtimes] stop failed:", result.reason);
      }
    }
  } catch (error) {
    console.warn("[managed-agent-runtimes] list failed:", error);
  }
}
