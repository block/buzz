import type { ManagedAgent } from "@/shared/api/types";

/**
 * Which agents are set up to run on Buzz shared compute.
 *
 * ## Why this reads `ManagedAgent`, not `AgentPersona`
 *
 * The mesh provider is usually set at the **global** layer
 * (`global-agent-config.json`), not per agent — a persona's own `provider`
 * field is null in that common case, so asking a persona "are you a mesh
 * agent?" answers "no" for every agent on a machine whose global default is
 * `relay-mesh`. That was the original bug here.
 *
 * Rust already resolves the layering (definition → global for linked
 * instances, instance → global for definition-less ones) in
 * `effective_config::resolve_effective_config`, and `runtime.rs` builds the
 * `ManagedAgent` DTO from that resolved value. So this file consumes the
 * resolved field and deliberately holds **no precedence rules of its own** —
 * a second copy of that layering would drift from the one in Rust.
 */

/** The provider id an agent carries when it runs on shared compute. */
export const RELAY_MESH_PROVIDER_ID = "relay-mesh";

/**
 * Whether an agent runs on shared compute.
 *
 * Takes the resolved `provider` off a `ManagedAgent`; never re-derives it.
 */
export function usesMeshCompute(
  agent: Pick<ManagedAgent, "provider"> | null | undefined,
): boolean {
  return agent?.provider?.trim() === RELAY_MESH_PROVIDER_ID;
}

/**
 * The card's call to action, or `none` when none is warranted.
 *
 * Deliberately silent in the common case. A nudge that appears every time the
 * card renders becomes furniture, so this only speaks when the setup is
 * genuinely incomplete in a way the person can act on: there is compute to use
 * and nothing set up to use it.
 */
export type MeshCallToAction =
  | { kind: "createAgent"; label: string }
  | { kind: "none" };

export function deriveMeshCallToAction({
  agents,
  isSharing,
  meshHasCapacity,
}: {
  /** Resolved agent list. `undefined` while still loading. */
  agents: ManagedAgent[] | undefined;
  isSharing: boolean;
  /** Anyone in the community is sharing, including this machine. */
  meshHasCapacity: boolean;
}): MeshCallToAction {
  // Unknown yet — say nothing rather than nudge toward a duplicate.
  if (agents === undefined) {
    return { kind: "none" };
  }
  if (agents.some(usesMeshCompute)) {
    return { kind: "none" };
  }
  // Compute with no consumer is the dead end worth naming. Only prompt when
  // there is actually capacity to use — telling someone to build an agent for
  // a mesh with nothing in it just moves the dead end.
  if (isSharing || meshHasCapacity) {
    return { kind: "createAgent", label: "Set up an agent to use it" };
  }
  return { kind: "none" };
}
