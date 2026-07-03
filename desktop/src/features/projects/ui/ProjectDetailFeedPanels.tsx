import {
  contributorKey,
  profileForCommitAuthor,
  profileForContributor,
} from "@/features/projects/lib/projectContributorMatching";
import type {
  ProjectRepoContributor,
  ProjectRepoSnapshot,
} from "@/features/projects/hooks";
import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { cn } from "@/shared/lib/cn";
import { ProfileIdentityButton } from "./ProjectProfileIdentity";

function compactDate(createdAt: number) {
  return new Date(createdAt * 1_000).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
}

function relativeCommitTime(createdAt: number) {
  const elapsedSeconds = Math.max(
    1,
    Math.floor(Date.now() / 1_000 - createdAt),
  );
  const units = [
    { label: "year", seconds: 365 * 24 * 60 * 60 },
    { label: "month", seconds: 30 * 24 * 60 * 60 },
    { label: "day", seconds: 24 * 60 * 60 },
    { label: "hour", seconds: 60 * 60 },
    { label: "min", seconds: 60 },
  ];
  const unit =
    units.find((item) => elapsedSeconds >= item.seconds) ??
    units[units.length - 1];
  const value = Math.max(1, Math.floor(elapsedSeconds / unit.seconds));
  return `${value} ${unit.label}${value === 1 ? "" : "s"} ago`;
}

function pluralize(count: number, singular: string, plural = `${singular}s`) {
  return `${count} ${count === 1 ? singular : plural}`;
}

export function ContributorsPanel({
  profiles,
  repoContributors,
}: {
  profiles?: UserProfileLookup;
  repoContributors: ProjectRepoContributor[];
}) {
  const rows = repoContributors.map((contributor) => {
    const matchedProfile = profileForContributor(contributor, profiles);
    const label = matchedProfile
      ? resolveUserLabel({ pubkey: matchedProfile.pubkey, profiles })
      : contributor.name || contributor.email || "Unknown contributor";

    return {
      avatarUrl: matchedProfile?.profile.avatarUrl ?? null,
      commitCount: contributor.commitCount,
      id: `git:${contributorKey(contributor)}`,
      isAgent: matchedProfile?.profile.isAgent === true,
      label,
      lastCommitAt: contributor.lastCommitAt,
      pubkey: matchedProfile?.pubkey ?? null,
      // Profile matches come from unauthenticated git author strings, so
      // they are surfaced as unverified rather than as a confirmed identity.
      role: matchedProfile
        ? `${
            matchedProfile.profile.nip05Handle ||
            contributor.email ||
            "Git contributor"
          } · unverified match`
        : contributor.email || "Git contributor",
    };
  });

  if (rows.length === 0) {
    return (
      <p className="rounded-xl border border-border/50 bg-card/60 p-4 text-sm text-muted-foreground">
        No git contributors are available yet.
      </p>
    );
  }

  return (
    <div className="overflow-hidden rounded-xl border border-border/50 bg-card/60">
      {rows.map((row, index) => (
        <div
          className={cn(
            "flex min-w-0 items-start gap-3 p-3 transition-colors hover:bg-muted/35",
            index !== rows.length - 1 && "border-border/50 border-b",
          )}
          key={row.id}
        >
          <ProfileIdentityButton
            avatarClassName="mt-0.5 shrink-0"
            avatarSize="md"
            avatarUrl={row.avatarUrl}
            isAgent={row.isAgent}
            label={row.label}
            pubkey={row.pubkey}
            showLabel={false}
          />
          <div className="min-w-0 flex-1 space-y-0.5">
            <p className="truncate text-sm font-semibold leading-5 text-foreground">
              {row.label}
            </p>
            <div className="flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-0.5 text-xs leading-4 text-muted-foreground">
              <span className="truncate">{row.role}</span>
              <span className="rounded-full border border-border/50 px-1.5 py-0.5 text-2xs">
                {row.commitCount === null
                  ? "No git commits"
                  : `${row.commitCount} commit${row.commitCount === 1 ? "" : "s"}`}
              </span>
              {row.lastCommitAt ? (
                <>
                  <span>·</span>
                  <span>updated {compactDate(row.lastCommitAt)}</span>
                </>
              ) : null}
            </div>
          </div>
        </div>
      ))}
    </div>
  );
}

export function ActivityPanel({
  snapshot,
  isLoading,
  error,
  profiles,
  repoContributors,
}: {
  snapshot: ProjectRepoSnapshot | null | undefined;
  isLoading: boolean;
  error: unknown;
  profiles?: UserProfileLookup;
  repoContributors: ProjectRepoContributor[];
}) {
  const commits = snapshot?.commits ?? [];

  if (isLoading) {
    return (
      <p className="p-4 text-sm text-muted-foreground">Loading activity…</p>
    );
  }

  if (commits.length === 0) {
    return (
      <p className="p-4 text-sm text-muted-foreground">
        {error
          ? "Could not load repository activity from git."
          : "No commits are available yet."}
      </p>
    );
  }

  return (
    <div className="relative">
      {commits.length > 1 ? (
        <div
          aria-hidden="true"
          className="absolute bottom-5 left-8 top-5 w-px bg-border/45"
        />
      ) : null}
      {commits.map((commit) => {
        const matchedProfile = profileForCommitAuthor(commit, profiles);
        const authorLabel = matchedProfile
          ? resolveUserLabel({ pubkey: matchedProfile.pubkey, profiles })
          : commit.authorName || commit.authorEmail || "Unknown author";
        const authorSubtitle = matchedProfile
          ? `${
              matchedProfile.profile.nip05Handle ||
              commit.authorEmail ||
              "Git contributor"
            } · unverified match`
          : commit.authorEmail || "Git contributor";
        const matchingContributor = repoContributors.find(
          (contributor) =>
            contributor.name.trim().toLowerCase() ===
              commit.authorName.trim().toLowerCase() ||
            contributor.email.trim().toLowerCase() ===
              commit.authorEmail.trim().toLowerCase(),
        );

        return (
          <article
            className="group/feed-item relative flex min-w-0 items-start gap-3 p-3 transition-colors hover:bg-muted/35"
            data-testid="project-activity-feed-item"
            key={commit.hash}
          >
            <ProfileIdentityButton
              avatarClassName="relative z-10 mt-0.5 shrink-0 ring-2 ring-card"
              avatarSize="md"
              avatarUrl={matchedProfile?.profile.avatarUrl ?? null}
              isAgent={matchedProfile?.profile.isAgent === true}
              label={authorLabel}
              pubkey={matchedProfile?.pubkey ?? null}
              showLabel={false}
            />
            <div className="min-w-0 flex-1 space-y-1.5">
              <div className="space-y-0.5">
                <div className="flex min-w-0 items-center gap-1.5">
                  <p className="truncate text-sm font-semibold leading-5 text-foreground">
                    {authorLabel}
                  </p>
                  <span className="shrink-0 text-sm leading-5 text-muted-foreground">
                    pushed a commit
                  </span>
                </div>
                <div className="flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-0.5 text-xs leading-4 text-muted-foreground">
                  <span className="truncate">{authorSubtitle}</span>
                  <span>{commit.shortHash}</span>
                  {matchingContributor?.commitCount ? (
                    <span className="rounded-full border border-border/50 px-1.5 py-0.5 text-2xs">
                      {pluralize(matchingContributor.commitCount, "commit")}
                    </span>
                  ) : null}
                  <span>·</span>
                  <time className="transition-colors group-hover/feed-item:text-foreground">
                    {relativeCommitTime(commit.timestamp)}
                  </time>
                </div>
              </div>
              <div className="rounded-lg border border-border/50 bg-background/45 px-3 py-1.5">
                <p className="line-clamp-2 text-sm font-medium leading-5 text-foreground">
                  {commit.subject}
                </p>
              </div>
            </div>
          </article>
        );
      })}
    </div>
  );
}
