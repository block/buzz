import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { WorkstreamIdentity } from "@/features/workstream-board/lib/workstreamCardParser";
import type { WorkstreamAgentStatuses } from "@/features/workstream-board/lib/workstreamAgentStatuses";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { AvatarStack } from "@/shared/ui/AvatarStack";
import { cn } from "@/shared/lib/cn";

function StatusPill({
  agents,
  label,
  profiles,
  status,
}: {
  agents: readonly WorkstreamIdentity[];
  label: string;
  profiles?: UserProfileLookup;
  status: "working" | "waiting";
}) {
  if (agents.length === 0) return null;
  const items = agents.map((agent) => {
    const profile = profiles?.[normalizePubkey(agent.pubkey)];
    return {
      id: normalizePubkey(agent.pubkey),
      avatarUrl: profile?.avatarUrl ?? null,
      displayName: profile?.displayName?.trim() || agent.name,
    };
  });
  return (
    <span
      className={cn(
        "inline-flex h-[1.875rem] items-center rounded-full border bg-background/70 py-0 pl-0.5 pr-2.5 text-xs font-medium",
        status === "working"
          ? "border-emerald-500/35 text-emerald-700 dark:text-emerald-200"
          : "border-amber-500/35 text-amber-800 dark:text-amber-200",
      )}
      data-testid={`workstream-agent-status-${status}`}
      title={items.map((item) => item.displayName).join(", ")}
    >
      <AvatarStack items={items} limit={4} testId="workstream-agent-avatar" />
      <span className="ml-1.5 whitespace-nowrap">
        {agents.length} {label}
      </span>
    </span>
  );
}

export function WorkstreamAgentStatusPills({
  profiles,
  statuses,
}: {
  profiles?: UserProfileLookup;
  statuses: WorkstreamAgentStatuses;
}) {
  if (statuses.working.length === 0 && statuses.waiting.length === 0)
    return null;
  return (
    <div
      className="flex flex-wrap gap-1.5"
      data-testid="workstream-agent-statuses"
    >
      <StatusPill
        agents={statuses.working}
        label="working"
        profiles={profiles}
        status="working"
      />
      <StatusPill
        agents={statuses.waiting}
        label="waiting"
        profiles={profiles}
        status="waiting"
      />
    </div>
  );
}
