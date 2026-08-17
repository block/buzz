const PROJECT_SIDEBAR_MEMBERSHIP_PREFIX = "buzz.sidebar.projects.membership.v1";
export const PROJECT_SIDEBAR_MEMBERSHIP_EVENT =
  "buzz:project-sidebar-membership-change";

function membershipKey(relayOrigin: string, pubkey: string) {
  return `${PROJECT_SIDEBAR_MEMBERSHIP_PREFIX}.${encodeURIComponent(relayOrigin)}.${pubkey.toLowerCase()}`;
}

export function readProjectSidebarMembership(
  relayOrigin: string | null | undefined,
  pubkey: string | null | undefined,
): string[] {
  if (!relayOrigin || !pubkey) return [];
  try {
    const parsed = JSON.parse(
      globalThis.localStorage?.getItem(membershipKey(relayOrigin, pubkey)) ??
        "[]",
    );
    return Array.isArray(parsed)
      ? [
          ...new Set(
            parsed.filter(
              (address): address is string =>
                typeof address === "string" && address.length > 0,
            ),
          ),
        ]
      : [];
  } catch {
    return [];
  }
}

function writeProjectSidebarMembership(
  relayOrigin: string,
  pubkey: string,
  addresses: readonly string[],
) {
  try {
    globalThis.localStorage?.setItem(
      membershipKey(relayOrigin, pubkey),
      JSON.stringify([...new Set(addresses)]),
    );
    globalThis.dispatchEvent?.(
      new CustomEvent(PROJECT_SIDEBAR_MEMBERSHIP_EVENT),
    );
  } catch {
    // Persistence is best-effort; callers still update their local view.
  }
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
