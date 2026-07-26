import type { MeshNodeStatus } from "@/shared/api/tauriMesh";

/**
 * Derived Share-compute toggle model.
 *
 * The single mesh runtime slot is shared by BOTH roles: serve mode (this
 * machine SHARING compute) and client mode (this machine CONSUMING a peer's
 * compute). Both report `state: "running"`. The Share toggle must therefore
 * key off `mode`, not `state` alone — otherwise consuming a peer's compute
 * lights up the Share switch and (worse) clicking it tears down the unrelated
 * client session. See `deriveMeshShareToggle`.
 */
export type MeshShareToggleModel = {
  /**
   * The Share switch is on: a serve-mode node occupies the slot. Stays true
   * while it is starting or even if it later fails health (the runtime still
   * occupies the slot, and the user must be able to turn it off to clear/retry
   * — see `StatusLine` for the health sub-state). A serve node that also routes
   * a peer's model is still sharing: routing is a capability, not a role.
   */
  isSharing: boolean;
  /**
   * A client-mode runtime occupies the single slot (this machine is consuming
   * a peer's compute). The Share switch must read off + disabled while true.
   */
  isConsuming: boolean;
  /**
   * ANY runtime occupies the single slot (serve or client, healthy or failed).
   * A fresh `mesh_start_node` fails with "already running" while true, so the
   * switch must not offer a start — only a stop of an existing serve node.
   */
  slotOccupied: boolean;
};

/**
 * A runtime object occupies the slot once it is starting or running — and also
 * when it has `failed` (it started, then errored; the runtime is still in the
 * slot and blocks a fresh start). `off`/`stopping` do not occupy it.
 */
function occupiesSlot(status: MeshNodeStatus | null): boolean {
  return (
    status?.state === "running" ||
    status?.state === "starting" ||
    status?.state === "failed"
  );
}

/**
 * Project a mesh node status into the Share toggle's on/consuming state.
 *
 * Pure and total (accepts `null` = status not yet fetched). This is the single
 * source of truth for "is this machine sharing?" — the component and its
 * regression tests both consume it so the `state`-only bug cannot come back.
 */
export function deriveMeshShareToggle(
  status: MeshNodeStatus | null,
): MeshShareToggleModel {
  const occupied = occupiesSlot(status);
  return {
    isSharing: occupied && status?.mode === "serve",
    isConsuming: occupied && status?.mode === "client",
    slotOccupied: occupied,
  };
}
