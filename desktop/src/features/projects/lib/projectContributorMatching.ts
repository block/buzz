import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type {
  ProjectRepoCommit,
  ProjectRepoContributor,
} from "@/shared/api/types";
import { parsePubkeyInput } from "@/shared/lib/nostrUtils";

type SignedProjectContributorSources = {
  owner: string;
  contributors: readonly string[];
  pullRequests: readonly {
    author: string;
    initialCommit?: string | null;
    commit?: string | null;
    comments: readonly { author: string }[];
    approvals: readonly { author: string }[];
    updates: readonly { author: string; commit: string | null }[];
  }[];
  issues: readonly {
    author: string;
    comments: readonly { author: string }[];
  }[];
};

export type ProjectContributorActivityCounts = {
  commits: number;
  reviews: number;
  tasks: number;
};

/**
 * Collects identities tied to a repository by its announcement or by signed
 * project activity. Assignments and recipients are intentionally excluded:
 * being asked to review or own a task is not itself a contribution.
 */
export function signedProjectContributorPubkeys({
  owner,
  contributors,
  pullRequests,
  issues,
}: SignedProjectContributorSources): string[] {
  return [
    ...new Set(
      [
        owner,
        ...contributors,
        ...pullRequests.flatMap((pullRequest) => [
          pullRequest.author,
          ...pullRequest.comments.map((comment) => comment.author),
          ...pullRequest.approvals.map((approval) => approval.author),
          ...pullRequest.updates.map((update) => update.author),
        ]),
        ...issues.flatMap((issue) => [
          issue.author,
          ...issue.comments.map((comment) => comment.author),
        ]),
      ]
        .filter(Boolean)
        .map((pubkey) => pubkey.trim().toLowerCase()),
    ),
  ];
}

/** Counts signed repository activity by actor for contributor list columns. */
export function projectContributorActivityCounts({
  owner,
  contributors,
  pullRequests,
  issues,
}: SignedProjectContributorSources): Record<
  string,
  ProjectContributorActivityCounts
> {
  const counts = Object.fromEntries(
    signedProjectContributorPubkeys({
      owner,
      contributors,
      pullRequests,
      issues,
    }).map((pubkey) => [pubkey, { commits: 0, reviews: 0, tasks: 0 }]),
  );
  const ensureCounts = (rawPubkey: string) => {
    const pubkey = rawPubkey.trim().toLowerCase();
    const existing = counts[pubkey];
    if (existing) return existing;
    const created = { commits: 0, reviews: 0, tasks: 0 };
    counts[pubkey] = created;
    return created;
  };

  for (const pubkey of commitAuthorPubkeysFromPullRequests(
    pullRequests,
  ).values()) {
    ensureCounts(pubkey).commits += 1;
  }
  for (const pullRequest of pullRequests) {
    ensureCounts(pullRequest.author).reviews += 1;
  }
  for (const issue of issues) {
    ensureCounts(issue.author).tasks += 1;
  }
  return counts;
}

export function contributorKey(contributor: ProjectRepoContributor) {
  return (contributor.email || contributor.name).trim().toLowerCase();
}

// Git author strings are unauthenticated — anyone can commit under any
// name/email — so a profile match here is a display heuristic, never an
// identity claim. Matching is limited to exact equality against profile
// fields; fuzzy matching (name prefixes, email local-parts) let arbitrary
// commit authors borrow a real user's avatar. Callers must present matches
// as unverified.
function profileMatchesContributor(
  contributor: ProjectRepoContributor,
  profile: UserProfileLookup[string] | undefined,
  pubkey?: string,
) {
  if (!profile) return false;
  const name = contributor.name.trim().toLowerCase();
  const email = contributor.email.trim().toLowerCase();
  const candidates = [
    pubkey,
    profile.displayName,
    profile.nip05Handle,
    profile.ownerPubkey,
  ]
    .map((value) => value?.trim().toLowerCase() ?? "")
    .filter(Boolean);

  return (
    (name.length > 0 && candidates.includes(name)) ||
    (email.length > 0 && candidates.includes(email))
  );
}

export function profileForContributor(
  contributor: ProjectRepoContributor,
  profiles: UserProfileLookup | undefined,
) {
  if (!profiles) return null;
  for (const [pubkey, profile] of Object.entries(profiles)) {
    if (profileMatchesContributor(contributor, profile, pubkey)) {
      return { pubkey, profile };
    }
  }
  return null;
}

export function profileForCommitAuthor(
  commit: ProjectRepoCommit,
  profiles: UserProfileLookup | undefined,
) {
  if (!profiles) return null;
  // A Git author may use their Nostr key as the name/email. This is still an
  // unauthenticated display hint—the signed PR mapping remains authoritative.
  const claimedPubkey =
    parsePubkeyInput(commit.authorName) ?? parsePubkeyInput(commit.authorEmail);
  if (claimedPubkey) {
    const profile = profiles[claimedPubkey];
    if (profile) {
      return { pubkey: claimedPubkey, profile };
    }
  }
  const contributor = {
    name: commit.authorName,
    email: commit.authorEmail,
    commitCount: 0,
    lastCommitAt: commit.timestamp,
  };
  for (const [pubkey, profile] of Object.entries(profiles)) {
    if (profileMatchesContributor(contributor, profile, pubkey)) {
      return { pubkey, profile };
    }
  }
  return null;
}

/**
 * Map commit hashes to the Nostr pubkey that published them through pull
 * request events. Unlike git author strings, this association is backed by a
 * signed relay event, so it is the preferred identity source for commits.
 */
export function commitAuthorPubkeysFromPullRequests(
  pullRequests: readonly {
    author: string;
    initialCommit?: string | null;
    commit?: string | null;
    updates?: readonly { author: string; commit: string | null }[];
  }[],
) {
  const byHash = new Map<string, string>();
  for (const pullRequest of pullRequests) {
    if (pullRequest.initialCommit) {
      byHash.set(pullRequest.initialCommit.toLowerCase(), pullRequest.author);
    }
    if (pullRequest.commit) {
      byHash.set(pullRequest.commit.toLowerCase(), pullRequest.author);
    }
    // Updates last: they carry the precise publisher for each pushed commit.
    for (const update of pullRequest.updates ?? []) {
      if (update.commit) {
        byHash.set(update.commit.toLowerCase(), update.author);
      }
    }
  }
  return byHash;
}

/**
 * Links aggregate Git author rows to Buzz identities through commit hashes
 * published in signed review events. Ambiguous author strings fail closed.
 */
export function gitContributorPubkeysFromCommits(
  commits: readonly ProjectRepoCommit[],
  pullRequests: Parameters<typeof commitAuthorPubkeysFromPullRequests>[0],
): Map<string, string> {
  const pubkeysByHash = commitAuthorPubkeysFromPullRequests(pullRequests);
  const candidatesByContributor = new Map<string, Set<string>>();
  for (const commit of commits) {
    const pubkey =
      pubkeysByHash.get(commit.hash.toLowerCase()) ??
      pubkeysByHash.get(commit.shortHash.toLowerCase());
    if (!pubkey) continue;
    const key = contributorKey({
      commitCount: 0,
      email: commit.authorEmail,
      lastCommitAt: commit.timestamp,
      name: commit.authorName,
    });
    const candidates = candidatesByContributor.get(key) ?? new Set<string>();
    candidates.add(pubkey.toLowerCase());
    candidatesByContributor.set(key, candidates);
  }

  return new Map(
    [...candidatesByContributor.entries()]
      .filter(([, pubkeys]) => pubkeys.size === 1)
      .map(([key, pubkeys]) => [key, [...pubkeys][0]]),
  );
}

/**
 * The viewer's own git identity (`git config user.name/user.email`) paired
 * with their Nostr pubkey. Only used to attribute the viewer's own commits —
 * their git author string is known locally, so this match doesn't rely on
 * profile fields lining up with the git config.
 */
export type ViewerGitIdentity = {
  pubkey: string | null;
  name?: string | null;
  email?: string | null;
};

function commitMatchesViewerGitIdentity(
  commit: ProjectRepoCommit,
  viewer: ViewerGitIdentity,
) {
  // Email-only equality. A display name is not an identity: two contributors
  // can share "Alex Chen", and a name-based match would borrow the viewer's
  // avatar for a stranger's commit. The git email is what the viewer's own
  // commits actually carry, so requiring it loses nothing legitimate.
  const email = commit.authorEmail.trim().toLowerCase();
  const viewerEmail = viewer.email?.trim().toLowerCase() ?? "";
  return email.length > 0 && email === viewerEmail;
}

/**
 * Resolve the best-known profile for a commit, in order of confidence:
 * 1. the signed PR-event mapping (verified publisher),
 * 2. the exact-match git author heuristic,
 * 3. the viewer's own git identity (local git config, display-only) — a last
 *    resort for the viewer's own commits when their git author string doesn't
 *    match any profile field.
 */
export function profileForCommit(
  commit: ProjectRepoCommit,
  profiles: UserProfileLookup | undefined,
  commitAuthorPubkeys?: Map<string, string>,
  viewerGitIdentity?: ViewerGitIdentity | null,
) {
  const mappedPubkey =
    commitAuthorPubkeys?.get(commit.hash.toLowerCase()) ??
    commitAuthorPubkeys?.get(commit.shortHash.toLowerCase());
  if (mappedPubkey) {
    const profile = profiles?.[mappedPubkey.toLowerCase()];
    if (profile) {
      return { pubkey: mappedPubkey.toLowerCase(), profile };
    }
  }
  const authorMatch = profileForCommitAuthor(commit, profiles);
  if (authorMatch) {
    return authorMatch;
  }
  if (
    viewerGitIdentity?.pubkey &&
    commitMatchesViewerGitIdentity(commit, viewerGitIdentity)
  ) {
    const pubkey = viewerGitIdentity.pubkey.toLowerCase();
    const profile = profiles?.[pubkey];
    if (profile) {
      return { pubkey, profile };
    }
  }
  return null;
}
