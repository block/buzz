import { ExternalLink } from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";

import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { WorkstreamPullRequestReference } from "@/features/workstream-board/lib/workstreamPullRequestStatus";
import type { WorkstreamWait } from "@/features/workstream-board/lib/workstreamWaits";
import { parseEntityLink } from "@/shared/lib/entityLink";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { useNow } from "@/shared/lib/useNow";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import { useOpenEntityLink } from "@/shared/ui/markdown/entityLinks";
import { cn } from "@/shared/lib/cn";

export function waitAge(since: string | undefined, now: number): string {
  const then = since ? Date.parse(since) : Number.NaN;
  if (!Number.isFinite(then)) return "Unknown";
  const minutes = Math.max(0, Math.floor((now - then) / 60_000));
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h`;
  return `${Math.floor(hours / 24)}d`;
}

function WaitAvatar({
  wait,
  profiles,
}: {
  wait: WorkstreamWait;
  profiles?: UserProfileLookup;
}) {
  const profile = wait.actor.pubkey
    ? profiles?.[normalizePubkey(wait.actor.pubkey)]
    : undefined;
  if (wait.actor.kind !== "team") {
    return (
      <UserAvatar
        avatarUrl={profile?.avatarUrl ?? null}
        className="size-[1.1875rem] text-3xs"
        displayName={profile?.displayName?.trim() || wait.actor.name}
        size="xs"
      />
    );
  }
  return (
    <span className="grid size-[1.1875rem] shrink-0 place-items-center rounded-[0.3rem] bg-violet-500/60 text-3xs font-bold text-white">
      {wait.actor.name.slice(0, 2).toUpperCase()}
    </span>
  );
}

export function WorkstreamWaits({
  profiles,
  references,
  waits,
}: {
  profiles?: UserProfileLookup;
  references: readonly WorkstreamPullRequestReference[];
  waits: readonly WorkstreamWait[];
}) {
  const now = useNow(60_000);
  const openEntityLink = useOpenEntityLink();
  if (waits.length === 0) return null;
  const referencesByIdentity = new Map(
    references.map((reference) => [reference.identity, reference]),
  );
  const orderedWaits = [...waits].sort((left, right) => {
    const leftAt = left.since
      ? Date.parse(left.since)
      : Number.POSITIVE_INFINITY;
    const rightAt = right.since
      ? Date.parse(right.since)
      : Number.POSITIVE_INFINITY;
    return leftAt - rightAt;
  });
  return (
    <section
      aria-label="Blockers"
      className="pointer-events-none mt-1 border-t border-border/60 pt-2"
      data-testid="workstream-waits"
    >
      <div className="mb-1.5 flex items-center justify-between gap-2 text-2xs text-muted-foreground">
        <p className="font-semibold text-amber-800 dark:text-amber-200">
          {waits.length} {waits.length === 1 ? "blocker" : "blockers"}
        </p>
        <span>oldest first</span>
      </div>
      <ul className="pointer-events-auto space-y-1">
        {orderedWaits.map((wait) => {
          const age = waitAge(wait.since, now);
          const reference = referencesByIdentity.get(wait.key);
          const open = reference
            ? () => {
                const parsed =
                  reference.kind === "buzz"
                    ? parseEntityLink(reference.href)
                    : null;
                if (parsed?.ok) openEntityLink(parsed.value);
                else void openUrl(reference.href);
              }
            : undefined;
          const content = (
            <>
              <WaitAvatar profiles={profiles} wait={wait} />
              <span className="min-w-0 flex-1 text-2xs leading-tight">
                <span
                  className="line-clamp-2"
                  title={`${wait.actor.name} · ${wait.reason}`}
                >
                  <span className="font-semibold">{wait.actor.name}</span>
                  {" · "}
                  {wait.reason}
                </span>
              </span>
              <span className="shrink-0 text-right text-3xs text-amber-800 dark:text-amber-200/80">
                <span className="block">{age}</span>
                <span className="block uppercase tracking-wide text-muted-foreground">
                  {wait.actor.kind} · {wait.source}
                </span>
              </span>
              {open ? (
                <ExternalLink className="size-3 shrink-0 text-muted-foreground" />
              ) : null}
            </>
          );
          const className = cn(
            "grid w-full grid-cols-[1.1875rem_minmax(0,1fr)_auto_auto] items-center gap-1.5 rounded-md border-l-2 px-1.5 py-1 text-left",
            wait.source === "derived"
              ? "border-l-sky-400 bg-sky-500/8"
              : "border-l-amber-400 bg-amber-500/10",
            open &&
              "transition-colors hover:bg-background/80 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring",
          );
          return (
            <li data-testid={`workstream-wait-${wait.key}`} key={wait.key}>
              {open ? (
                <button className={className} onClick={open} type="button">
                  {content}
                </button>
              ) : (
                <div className={className}>{content}</div>
              )}
            </li>
          );
        })}
      </ul>
    </section>
  );
}
