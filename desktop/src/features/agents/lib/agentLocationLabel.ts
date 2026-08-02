import type { ManagedAgentBackend } from "@/shared/api/types";
import { backendProviderLabel } from "./backendProviderLabel";

/**
 * Where an agent runs, for the surfaces that list agents.
 *
 * `null` for a local agent, on purpose and everywhere. "On this computer" is
 * the default a user already assumes, so painting it on every card would cost a
 * line of metadata to say nothing; the label exists to mark the records that
 * are NOT here.
 *
 * The label names the PROVIDER, never the host. `backend.config` holds the
 * host, but reading a blessed `ssh_host` key out of it is the thing
 * `exclusiveRemoteHarness` explicitly refuses to do: the desktop has no
 * vocabulary for "the host field" and would have to grow one per provider.
 * Naming the provider is the honest answer this record can give without the
 * desktop learning any provider's config shape.
 */
export function agentRunsOnLabel(
  backend: ManagedAgentBackend | undefined | null,
): string | null {
  if (backend?.type !== "provider") return null;
  return backendProviderLabel(backend.id);
}

/**
 * `"on ssh"` — the same fact as a compact metadata line for an agent card.
 *
 * Unprobed ids are the label, matching `runTargetOptions`, which renders a
 * discovered provider as its id until a probe has paid for a friendlier name.
 * A card list is exactly the place that must not spawn one subprocess per
 * provider to decorate a string.
 */
export function agentLocationLabel(
  backend: ManagedAgentBackend | undefined | null,
): string | null {
  const name = agentRunsOnLabel(backend);
  return name ? `on ${name}` : null;
}
