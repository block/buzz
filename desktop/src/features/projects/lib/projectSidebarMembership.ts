const PROJECT_SIDEBAR_MEMBERSHIP_PREFIX = "buzz.sidebar.projects.membership.v1";
export const PROJECT_SIDEBAR_MEMBERSHIP_EVENT =
  "buzz:project-sidebar-membership-change";

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

export function readProjectSidebarMembershipStore(
  relayOrigin: string | null | undefined,
  pubkey: string | null | undefined,
): ProjectSidebarMembershipStore {
  if (!relayOrigin || !pubkey) return EMPTY_PROJECT_SIDEBAR_MEMBERSHIP_STORE;
  try {
    const raw = globalThis.localStorage?.getItem(
      projectSidebarMembershipStorageKey(relayOrigin, pubkey),
    );
    if (!raw) return EMPTY_PROJECT_SIDEBAR_MEMBERSHIP_STORE;
    return (
      parseProjectSidebarMembershipPayload(JSON.parse(raw)) ??
      EMPTY_PROJECT_SIDEBAR_MEMBERSHIP_STORE
    );
  } catch {
    return EMPTY_PROJECT_SIDEBAR_MEMBERSHIP_STORE;
  }
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

export function writeProjectSidebarMembershipStore(
  relayOrigin: string,
  pubkey: string,
  store: ProjectSidebarMembershipStore,
  notify = true,
): boolean {
  try {
    globalThis.localStorage?.setItem(
      projectSidebarMembershipStorageKey(relayOrigin, pubkey),
      JSON.stringify(store),
    );
    if (notify) {
      globalThis.dispatchEvent?.(
        new CustomEvent(PROJECT_SIDEBAR_MEMBERSHIP_EVENT),
      );
    }
    return true;
  } catch {
    return false;
  }
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
