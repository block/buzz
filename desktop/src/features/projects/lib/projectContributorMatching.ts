import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type {
  ProjectRepoCommit,
  ProjectRepoContributor,
} from "@/shared/api/types";

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
  const name = commit.authorName.trim().toLowerCase();
  const email = commit.authorEmail.trim().toLowerCase();
  const viewerName = viewer.name?.trim().toLowerCase() ?? "";
  const viewerEmail = viewer.email?.trim().toLowerCase() ?? "";
  return (
    (email.length > 0 && email === viewerEmail) ||
    (name.length > 0 && name === viewerName)
  );
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
