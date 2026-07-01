import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type {
  ProjectRepoCommit,
  ProjectRepoContributor,
} from "@/shared/api/types";

export function contributorKey(contributor: ProjectRepoContributor) {
  return (contributor.email || contributor.name).trim().toLowerCase();
}

function profileMatchesContributor(
  contributor: ProjectRepoContributor,
  profile: UserProfileLookup[string] | undefined,
  pubkey?: string,
) {
  if (!profile) return false;
  const name = contributor.name.trim().toLowerCase();
  const email = contributor.email.trim().toLowerCase();
  const emailLocalPart = email.split("@")[0] ?? "";
  const candidates = [
    pubkey,
    profile.displayName,
    profile.nip05Handle,
    profile.ownerPubkey,
  ]
    .map((value) => value?.trim().toLowerCase() ?? "")
    .filter(Boolean);
  const candidateLocalParts = candidates.map(
    (candidate) => candidate.split("@")[0] ?? "",
  );

  return (
    candidates.includes(name) ||
    candidates.includes(email) ||
    candidateLocalParts.includes(emailLocalPart) ||
    candidates.some(
      (candidate) => candidate.length >= 4 && name.startsWith(candidate),
    )
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
