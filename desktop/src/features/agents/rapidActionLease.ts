export type RapidActionLeaseScope = {
  agentPubkey: string;
  activeRelayUrl: string | null;
  ownerIdentityPubkey: string | null;
  route: string;
};

export type RapidActionLease = {
  id: string;
  controller: AbortController;
};

type LeaseEntry = RapidActionLease & {
  agentPubkey: string;
  scope: RapidActionLeaseScope;
  mounts: Set<string>;
};

const leasesById = new Map<string, LeaseEntry>();
const leaseIdByAgent = new Map<string, string>();
const leaseIdByMount = new Map<string, string>();
let nextLeaseId = 0;
let nextMountId = 0;

function normalizeScope(scope: RapidActionLeaseScope): RapidActionLeaseScope {
  return {
    agentPubkey: scope.agentPubkey.trim().toLowerCase(),
    activeRelayUrl: scope.activeRelayUrl?.trim() || null,
    ownerIdentityPubkey: scope.ownerIdentityPubkey?.trim().toLowerCase() || null,
    route: scope.route,
  };
}

function scopesMatch(
  left: RapidActionLeaseScope,
  right: RapidActionLeaseScope,
): boolean {
  return (
    left.agentPubkey === right.agentPubkey &&
    left.activeRelayUrl === right.activeRelayUrl &&
    left.ownerIdentityPubkey === right.ownerIdentityPubkey &&
    left.route === right.route
  );
}

export function createRapidActionMountId(): string {
  nextMountId += 1;
  return `rapid-mount-${nextMountId}`;
}

/** Start one action lease for an agent, owned by the current dialog mount. */
export function startRapidActionLease(
  scope: RapidActionLeaseScope,
  mountId: string,
): RapidActionLease | null {
  const normalizedScope = normalizeScope(scope);
  if (leaseIdByAgent.has(normalizedScope.agentPubkey)) {
    return null;
  }

  nextLeaseId += 1;
  const lease: LeaseEntry = {
    id: `rapid-action-${nextLeaseId}`,
    controller: new AbortController(),
    agentPubkey: normalizedScope.agentPubkey,
    scope: normalizedScope,
    mounts: new Set([mountId]),
  };
  leasesById.set(lease.id, lease);
  leaseIdByAgent.set(lease.agentPubkey, lease.id);
  leaseIdByMount.set(mountId, lease.id);
  return lease;
}

/**
 * Reclaim a lease only during a same-scope mount handoff. A second dialog
 * cannot attach to an action while the original owner is still mounted.
 */
export function claimRapidActionLease(
  scope: RapidActionLeaseScope,
  mountId: string,
): RapidActionLease | null {
  const normalizedScope = normalizeScope(scope);
  const leaseId = leaseIdByAgent.get(normalizedScope.agentPubkey);
  if (!leaseId) {
    return null;
  }
  const lease = leasesById.get(leaseId);
  if (
    !lease ||
    lease.controller.signal.aborted ||
    lease.mounts.size > 0 ||
    !scopesMatch(lease.scope, normalizedScope)
  ) {
    return null;
  }

  lease.mounts.add(mountId);
  leaseIdByMount.set(mountId, lease.id);
  return lease;
}

/** Release a dialog mount; abort if no same-scope replacement claims it. */
export function releaseRapidActionMount(mountId: string): void {
  const leaseId = leaseIdByMount.get(mountId);
  if (!leaseId) {
    return;
  }
  leaseIdByMount.delete(mountId);
  const lease = leasesById.get(leaseId);
  if (!lease) {
    return;
  }
  lease.mounts.delete(mountId);
  globalThis.queueMicrotask(() => {
    const current = leasesById.get(leaseId);
    if (current && current.mounts.size === 0) {
      current.controller.abort();
    }
  });
}

export function abortRapidActionLeaseForMount(mountId: string): void {
  const leaseId = leaseIdByMount.get(mountId);
  if (!leaseId) {
    return;
  }
  leasesById.get(leaseId)?.controller.abort();
}

export function finishRapidActionLease(leaseId: string): void {
  const lease = leasesById.get(leaseId);
  if (!lease) {
    return;
  }
  for (const mountId of lease.mounts) {
    leaseIdByMount.delete(mountId);
  }
  if (leaseIdByAgent.get(lease.agentPubkey) === leaseId) {
    leaseIdByAgent.delete(lease.agentPubkey);
  }
  leasesById.delete(leaseId);
}

/** Community switches are a hard authority boundary for all rapid actions. */
export function resetRapidActionLeases(): void {
  for (const lease of leasesById.values()) {
    lease.controller.abort();
  }
  leasesById.clear();
  leaseIdByAgent.clear();
  leaseIdByMount.clear();
}
