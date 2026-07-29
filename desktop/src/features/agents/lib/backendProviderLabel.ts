import type { BackendProviderCandidate } from "@/shared/api/types";

/**
 * What every surface says when no provider is installed.
 *
 * One constant rather than the same sentence typed into the create dialog, the
 * onboarding notice and the Settings gallery: a user meets this line in up to
 * three places, and three spellings of one fact read as three different facts.
 */
export const NO_BACKEND_PROVIDER_HINT =
  "Install a backend provider to run agents on another machine.";

/**
 * How the app names a backend provider — the `buzz-backend-*` binary that runs
 * an agent on a machine other than this computer.
 *
 * A provider's own `info.name` ("SSH") is friendlier than its binary-derived id
 * ("ssh"), but `info` is a subprocess round-trip: surfaces that render a
 * provider before the user has asked anything of it (the onboarding notice, an
 * agent card) have not paid for one, so the id stands in. That is the same
 * trade `runTargetOptions` makes in the create dialog, and this is the single
 * owner of the rule so the two cannot drift into different naming schemes for
 * the same machine.
 */
export function backendProviderLabel(
  id: string,
  probedName?: string | null,
): string {
  const name = probedName?.trim();
  if (name) return name;
  const trimmedId = id.trim();
  return trimmedId || "Unknown provider";
}

/**
 * Labels for a discovered provider list, in a stable order.
 *
 * Sorted rather than left in discovery order because discovery walks `PATH`,
 * so the same two providers can come back in a different order between reads
 * and a hint line would reshuffle itself under the user.
 *
 * Ids only: its one caller is the onboarding notice, which renders before the
 * user has asked anything of a provider and so has not paid for a probe. A
 * surface that HAS probed names its rows through `backendProviderLabel`
 * directly with the name it already holds (`remoteServerEntries`).
 */
export function backendProviderLabels(
  providers: readonly BackendProviderCandidate[],
): string[] {
  return providers
    .map((provider) => backendProviderLabel(provider.id))
    .sort((left, right) => left.localeCompare(right));
}
