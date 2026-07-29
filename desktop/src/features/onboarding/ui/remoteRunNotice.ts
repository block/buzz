import {
  backendProviderLabels,
  NO_BACKEND_PROVIDER_HINT,
} from "@/features/agents/lib/backendProviderLabel";
import type { BackendProviderCandidate } from "@/shared/api/types";

/**
 * What the setup step says about the OTHER place an agent can run.
 *
 * The step itself only configures this computer's harnesses — a server is
 * chosen per-agent in the create dialog — so this is a statement about
 * capability, never a control. Three states rather than two because provider
 * discovery is async: rendering the install hint while the walk is still in
 * flight would tell a user with a provider installed that they have none, then
 * silently contradict itself a frame later.
 */
export type RemoteRunNotice =
  | { kind: "pending" }
  | { kind: "ready"; message: string }
  | { kind: "hint"; message: string };

/**
 * Project discovered providers into the setup step's location notice.
 *
 * `discoverBackendProviders` is a PATH walk, not a provider spawn, so this
 * reads names from ids alone (`backendProviderLabel`) rather than probing every
 * discovered binary to decorate a line the user did not ask for.
 */
export function remoteRunNotice(input: {
  isLoading: boolean;
  providers: readonly BackendProviderCandidate[] | undefined;
}): RemoteRunNotice {
  if (input.isLoading) return { kind: "pending" };

  const providerLabels = backendProviderLabels(input.providers ?? []);
  if (providerLabels.length === 0) {
    return { kind: "hint", message: NO_BACKEND_PROVIDER_HINT };
  }

  return {
    kind: "ready",
    message: `${providerLabels.join(", ")} detected — pick a server when you create an agent.`,
  };
}
