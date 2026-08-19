const PROJECT_SIDEBAR_MEMBERSHIP_PREFIX = "buzz.sidebar.projects.membership.v1";
export const PROJECT_SIDEBAR_MEMBERSHIP_EVENT =
  "buzz:project-sidebar-membership-change";

/** Detail carried by {@link PROJECT_SIDEBAR_MEMBERSHIP_EVENT}: the computed
 * membership for one relay/pubkey scope. Listeners must consume this rather
 * than re-reading localStorage — when persistence fails the write never
 * lands, and re-reading would silently revert the user's add/remove. */
export type ProjectSidebarMembershipChange = {
  relayOrigin: string;
  pubkey: string;
  addresses: string[];
};

function membershipKey(relayOrigin: string, pubkey: string) {
  return `${PROJECT_SIDEBAR_MEMBERSHIP_PREFIX}.${encodeURIComponent(relayOrigin)}.${pubkey.toLowerCase()}`;
}

/**
 * Scope-keyed authoritative membership. localStorage is only the durable
 * mirror: once a scope is seeded here, every read and mutation goes through
 * this map, so `add(A) → add(B) → remove(A)` accumulates correctly even when
 * every storage write fails — recomputing each mutation from storage would
 * silently drop all but the latest unpersisted change.
 */
const membershipByScope = new Map<string, string[]>();

/** Clears the in-memory authoritative scopes between test cases. */
export function __resetProjectSidebarMembershipForTests(): void {
  membershipByScope.clear();
}

function dedupe(addresses: readonly unknown[]): string[] {
  return [
    ...new Set(
      addresses.filter(
        (address): address is string =>
          typeof address === "string" && address.length > 0,
      ),
    ),
  ];
}

function readStoredMembership(key: string): string[] {
  try {
    const parsed = JSON.parse(globalThis.localStorage?.getItem(key) ?? "[]");
    return Array.isArray(parsed) ? dedupe(parsed) : [];
  } catch {
    return [];
  }
}

export function readProjectSidebarMembership(
  relayOrigin: string | null | undefined,
  pubkey: string | null | undefined,
): string[] {
  if (!relayOrigin || !pubkey) return [];
  const key = membershipKey(relayOrigin, pubkey);
  let addresses = membershipByScope.get(key);
  if (!addresses) {
    addresses = readStoredMembership(key);
    membershipByScope.set(key, addresses);
  }
  return [...addresses];
}

function writeProjectSidebarMembership(
  relayOrigin: string,
  pubkey: string,
  addresses: readonly string[],
) {
  const deduped = dedupe(addresses);
  membershipByScope.set(membershipKey(relayOrigin, pubkey), deduped);
  try {
    globalThis.localStorage?.setItem(
      membershipKey(relayOrigin, pubkey),
      JSON.stringify(deduped),
    );
  } catch {
    // Persistence is best-effort; the in-memory scope above stays
    // authoritative, so sequential add/remove never loses earlier
    // unpersisted changes, and the event below updates every mounted view.
  }
  globalThis.dispatchEvent?.(
    new CustomEvent<ProjectSidebarMembershipChange>(
      PROJECT_SIDEBAR_MEMBERSHIP_EVENT,
      { detail: { addresses: deduped, pubkey, relayOrigin } },
    ),
  );
}

export function addProjectToSidebar(
  projectAddress: string,
  relayOrigin: string | null | undefined,
  pubkey: string | null | undefined,
) {
  if (!relayOrigin || !pubkey) return;
  writeProjectSidebarMembership(relayOrigin, pubkey, [
    ...readProjectSidebarMembership(relayOrigin, pubkey),
    projectAddress,
  ]);
}

export function removeProjectFromSidebar(
  projectAddress: string,
  relayOrigin: string | null | undefined,
  pubkey: string | null | undefined,
) {
  if (!relayOrigin || !pubkey) return;
  writeProjectSidebarMembership(
    relayOrigin,
    pubkey,
    readProjectSidebarMembership(relayOrigin, pubkey).filter(
      (address) => address !== projectAddress,
    ),
  );
}
