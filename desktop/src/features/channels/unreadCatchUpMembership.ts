import type { ObservedUnreadMembershipUpdate } from "@/shared/api/tauriObservedUnread";
import type { UnreadCatchUpChannelResult } from "@/shared/api/tauriUnreadCatchUp";

type DiscoveredRoots = Extract<
  UnreadCatchUpChannelResult,
  { status: "success" }
>["discovered"];
type MembershipKind = keyof DiscoveredRoots;

export function applyCatchUpDiscoveries(
  discovered: DiscoveredRoots,
  sets: Record<MembershipKind, Set<string>>,
): { updates: ObservedUnreadMembershipUpdate[]; didDiscover: boolean } {
  const updates: ObservedUnreadMembershipUpdate[] = [];
  let didDiscover = false;
  for (const kind of ["participated", "authored", "mentioned"] as const) {
    for (const value of discovered[kind]) {
      if (!sets[kind].has(value)) didDiscover = true;
      sets[kind].add(value);
      updates.push({ kind, value, present: true });
    }
  }
  return { updates, didDiscover };
}
