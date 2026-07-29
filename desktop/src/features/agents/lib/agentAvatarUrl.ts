import type { ManagedAgent } from "@/shared/api/types";
import { providerRecordHarness } from "./pinnedHarness";

/**
 * The image that stands for an agent on the surfaces that list agents.
 *
 * Precedence is by authorship: what a human chose for this agent, then what
 * the agent published about itself, then the mark of the harness it runs.
 * Nothing here invents an identity — the last step only names the harness the
 * record is already pinned to.
 *
 * The harness step exists because it is what a local agent has always had.
 * Creating one stamps the local runtime's avatar onto the record, which is the
 * only reason a Claude card shows the Claude mark. A provider-backed record
 * gets no such stamp: its harness lives on the host, and the catalog entry the
 * host advertises deliberately carries no avatar url (rendering a
 * host-supplied image would be a tracking-pixel and spoofing vector — the
 * bundled logo maps are the only permitted route, see `RuntimeIcon`). So a
 * remote agent fell through to blank initials while its local twin showed a
 * logo, for reasons that were never about the agent.
 *
 * Deriving it at render time rather than at deploy time is deliberate: the
 * fleet already exists, and records minted before this fix carry an empty
 * avatar forever.
 */
export function resolveAgentAvatarUrl({
  agent,
  personaAvatarUrl,
  profileAvatarUrl,
  recordAvatarUrl,
}: {
  /** The record, or `undefined` for a persona that has never been spawned. */
  agent:
    | Pick<ManagedAgent, "backend" | "agentCommand" | "agentArgs">
    | undefined;
  /** The avatar on the agent's definition, chosen by a human. */
  personaAvatarUrl?: string | null;
  /** The avatar the running agent published about itself. */
  profileAvatarUrl?: string | null;
  /**
   * The avatar stamped onto the record when it was created — this computer's
   * runtime avatar for a local agent, and empty for a remote one, which is the
   * gap the harness mark below fills.
   */
  recordAvatarUrl?: string | null;
}): string | null {
  const chosen = firstNonEmpty(
    personaAvatarUrl,
    profileAvatarUrl,
    recordAvatarUrl,
  );
  if (chosen) return chosen;
  // `null` for a local record, whose avatar was already stamped from this
  // computer's catalog at create time — its rendering must not change.
  return agent ? (providerRecordHarness(agent)?.logoUrl ?? null) : null;
}

function firstNonEmpty(
  ...candidates: Array<string | null | undefined>
): string | null {
  for (const candidate of candidates) {
    const trimmed = candidate?.trim();
    if (trimmed) return trimmed;
  }
  return null;
}
