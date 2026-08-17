import { Users } from "lucide-react";
import { useMemo } from "react";

import type { Repo } from "../use-repos";
import { ConnectButton } from "./ConnectButton";
import { PubkeyAvatar } from "./PubkeyAvatar";

const MAX_AVATARS = 20;

export function OrgSidebar({ repos }: { repos: Repo[] }) {
  const uniquePubkeys = useMemo(() => {
    const set = new Set<string>();
    for (const repo of repos) {
      set.add(repo.owner);
      for (const c of repo.contributors) {
        set.add(c);
      }
    }
    return [...set];
  }, [repos]);

  const visiblePubkeys = uniquePubkeys.slice(0, MAX_AVATARS);
  const overflowCount = uniquePubkeys.length - MAX_AVATARS;

  return (
    <div className="space-y-6">
      {/* Open in Buzz */}
      <ConnectButton className="w-full" />

      {/* People section */}
      {uniquePubkeys.length > 0 && (
        <div>
          <h3 className="mb-3 flex items-center gap-2 text-sm font-semibold text-black dark:text-white">
            <Users className="h-4 w-4" />
            People
          </h3>
          <div className="flex flex-wrap gap-2">
            {visiblePubkeys.map((pk) => (
              <PubkeyAvatar key={pk} pubkey={pk} />
            ))}
          </div>
          {overflowCount > 0 && (
            <span className="mt-2 block text-xs text-black/50 dark:text-white/50">
              {uniquePubkeys.length} people
            </span>
          )}
        </div>
      )}
    </div>
  );
}
