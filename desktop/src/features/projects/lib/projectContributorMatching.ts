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
