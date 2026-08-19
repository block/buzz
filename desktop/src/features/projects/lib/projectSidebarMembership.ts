const PROJECT_SIDEBAR_MEMBERSHIP_PREFIX = "buzz.sidebar.projects.membership.v1";
export const PROJECT_SIDEBAR_MEMBERSHIP_EVENT =
  "buzz:project-sidebar-membership-change";

/** Detail carried by {@link PROJECT_SIDEBAR_MEMBERSHIP_EVENT}: the computed
 * membership for one relay/pubkey scope. Listeners may consume this instead of
 * re-reading storage — when persistence fails the write never lands, but the
 * in-session scope below stays authoritative either way. */
export type ProjectSidebarMembershipChange = {
  relayOrigin: string;
  pubkey: string;
  addresses: string[];
};

export type ProjectSidebarMembershipEntry = {
  selected: boolean;
  updatedAt: number;
};

export type ProjectSidebarMembershipStore = {
  version: 1;
  projects: Record<string, ProjectSidebarMembershipEntry>;
};

export const EMPTY_PROJECT_SIDEBAR_MEMBERSHIP_STORE: ProjectSidebarMembershipStore =
  Object.freeze({
    version: 1,
    projects: {},
  });

export function projectSidebarMembershipStorageKey(
  relayOrigin: string,
  pubkey: string,
) {
  return `${PROJECT_SIDEBAR_MEMBERSHIP_PREFIX}.${encodeURIComponent(relayOrigin)}.${pubkey.toLowerCase()}`;
}

export function parseProjectSidebarMembershipPayload(
  value: unknown,
): ProjectSidebarMembershipStore | null {
  if (Array.isArray(value)) {
    return {
      version: 1,
      projects: Object.fromEntries(
        value
          .filter(
            (address): address is string =>
              typeof address === "string" && address.length > 0,
          )
          .map((address) => [
            address,
            {
              selected: true,
              updatedAt: 0,
            } satisfies ProjectSidebarMembershipEntry,
          ]),
      ),
    };
  }
  if (!value || typeof value !== "object") return null;
  const candidate = value as Record<string, unknown>;
  if (candidate.version !== 1) return null;
  if (
    !candidate.projects ||
    typeof candidate.projects !== "object" ||
    Array.isArray(candidate.projects)
  ) {
    return null;
  }
  const projects = Object.fromEntries(
    Object.entries(candidate.projects).filter(
      (entry): entry is [string, ProjectSidebarMembershipEntry] => {
        const membership = entry[1];
        return (
          entry[0].length > 0 &&
          typeof membership === "object" &&
          membership !== null &&
          typeof (membership as Record<string, unknown>).selected ===
            "boolean" &&
          typeof (membership as Record<string, unknown>).updatedAt ===
            "number" &&
          Number.isFinite(
            (membership as Record<string, unknown>).updatedAt as number,
          ) &&
          ((membership as Record<string, unknown>).updatedAt as number) >= 0
        );
      },
    ),
  );
  return { version: 1, projects };
}

/**
 * Scope-keyed authoritative membership. localStorage is only the durable
 * mirror: once a scope is seeded here, every read and mutation goes through
 * this map, so `add(A) → add(B) → remove(A)` accumulates correctly even when
 * every storage write fails — recomputing each mutation from storage would
 * silently drop all but the latest unpersisted change. Scoping by
 * relay origin and pubkey keeps entries from crossing tenant boundaries.
 */
const membershipStoreByScope = new Map<string, ProjectSidebarMembershipStore>();

/** Clears the in-memory authoritative scopes between test cases. */
export function __resetProjectSidebarMembershipForTests(): void {
  membershipStoreByScope.clear();
}

function readStoredMembershipStore(key: string): ProjectSidebarMembershipStore {
  try {
    const raw = globalThis.localStorage?.getItem(key);
    if (!raw) return EMPTY_PROJECT_SIDEBAR_MEMBERSHIP_STORE;
    return (
      parseProjectSidebarMembershipPayload(JSON.parse(raw)) ??
      EMPTY_PROJECT_SIDEBAR_MEMBERSHIP_STORE
    );
  } catch {
    return EMPTY_PROJECT_SIDEBAR_MEMBERSHIP_STORE;
  }
}

export function readProjectSidebarMembershipStore(
  relayOrigin: string | null | undefined,
  pubkey: string | null | undefined,
): ProjectSidebarMembershipStore {
  if (!relayOrigin || !pubkey) return EMPTY_PROJECT_SIDEBAR_MEMBERSHIP_STORE;
  const key = projectSidebarMembershipStorageKey(relayOrigin, pubkey);
  const cached = membershipStoreByScope.get(key);
  const stored = readStoredMembershipStore(key);
  if (!cached) {
    membershipStoreByScope.set(key, stored);
    return stored;
  }
  // Merge instead of preferring either side: the cache holds mutations whose
  // persistence failed, while storage may hold newer cross-tab writes. The
  // per-entry LWW merge keeps both.
  if (projectSidebarMembershipStoresEqual(cached, stored)) return cached;
  const merged = mergeProjectSidebarMembershipStores(cached, stored);
  membershipStoreByScope.set(key, merged);
  return merged;
}

export function selectedProjectAddressesFromStore(
  store: ProjectSidebarMembershipStore,
): string[] {
  return Object.entries(store.projects)
    .filter(([, membership]) => membership.selected)
    .map(([address]) => address);
}

export function readProjectSidebarMembership(
  relayOrigin: string | null | undefined,
  pubkey: string | null | undefined,
): string[] {
  return selectedProjectAddressesFromStore(
    readProjectSidebarMembershipStore(relayOrigin, pubkey),
  );
}

export function mergeProjectSidebarMembershipStores(
  local: ProjectSidebarMembershipStore,
  remote: ProjectSidebarMembershipStore,
): ProjectSidebarMembershipStore {
  const addresses = new Set([
    ...Object.keys(local.projects),
    ...Object.keys(remote.projects),
  ]);
  const projects: Record<string, ProjectSidebarMembershipEntry> = {};
  for (const address of addresses) {
    const localEntry = local.projects[address];
    const remoteEntry = remote.projects[address];
    if (localEntry && remoteEntry) {
      if (localEntry.updatedAt > remoteEntry.updatedAt) {
        projects[address] = localEntry;
      } else if (remoteEntry.updatedAt > localEntry.updatedAt) {
        projects[address] = remoteEntry;
      } else {
        projects[address] =
          localEntry.selected === remoteEntry.selected
            ? localEntry
            : { selected: false, updatedAt: localEntry.updatedAt };
      }
    } else {
      projects[address] = (localEntry ??
        remoteEntry) as ProjectSidebarMembershipEntry;
    }
  }
  return { version: 1, projects };
}

export function projectSidebarMembershipStoresEqual(
  left: ProjectSidebarMembershipStore,
  right: ProjectSidebarMembershipStore,
): boolean {
  const leftKeys = Object.keys(left.projects);
  const rightKeys = Object.keys(right.projects);
  if (leftKeys.length !== rightKeys.length) return false;
  return leftKeys.every((address) => {
    const leftEntry = left.projects[address];
    const rightEntry = right.projects[address];
    return (
      rightEntry !== undefined &&
      leftEntry.selected === rightEntry.selected &&
      leftEntry.updatedAt === rightEntry.updatedAt
    );
  });
}

/**
 * Records `store` as the authoritative in-session state for the scope, then
 * mirrors it to localStorage. Persistence is best-effort: on failure the
 * in-memory scope stays authoritative, so sequential mutations never lose
 * earlier unpersisted changes and the next successful write persists the
 * accumulated state. Returns whether the mirror write landed.
 */
export function writeProjectSidebarMembershipStore(
  relayOrigin: string,
  pubkey: string,
  store: ProjectSidebarMembershipStore,
  notify = true,
): boolean {
  const key = projectSidebarMembershipStorageKey(relayOrigin, pubkey);
  membershipStoreByScope.set(key, store);
  let persisted = true;
  try {
    globalThis.localStorage?.setItem(key, JSON.stringify(store));
  } catch {
    persisted = false;
  }
  if (notify) {
    globalThis.dispatchEvent?.(
      new CustomEvent<ProjectSidebarMembershipChange>(
        PROJECT_SIDEBAR_MEMBERSHIP_EVENT,
        {
          detail: {
            addresses: selectedProjectAddressesFromStore(store),
            pubkey,
            relayOrigin,
          },
        },
      ),
    );
  }
  return persisted;
}

export function addProjectToSidebar(
  projectAddress: string,
  relayOrigin: string | null | undefined,
  pubkey: string | null | undefined,
) {
  if (!relayOrigin || !pubkey) return;
  const current = readProjectSidebarMembershipStore(relayOrigin, pubkey);
  writeProjectSidebarMembershipStore(relayOrigin, pubkey, {
    version: 1,
    projects: {
      ...current.projects,
      [projectAddress]: { selected: true, updatedAt: Date.now() },
    },
  });
}

export function removeProjectFromSidebar(
  projectAddress: string,
  relayOrigin: string | null | undefined,
  pubkey: string | null | undefined,
) {
  if (!relayOrigin || !pubkey) return;
  const current = readProjectSidebarMembershipStore(relayOrigin, pubkey);
  writeProjectSidebarMembershipStore(relayOrigin, pubkey, {
    version: 1,
    projects: {
      ...current.projects,
      [projectAddress]: { selected: false, updatedAt: Date.now() },
    },
  });
}
