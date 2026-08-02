const REPO_LINK_SCHEME = "buzz:";
const REPO_LINK_HOST = "repo";
const REPO_ID_PATTERN = /^(?!\.)(?!.*\.\.)[A-Za-z0-9._-]{1,64}$/;
const HEX_64_PATTERN = /^[0-9a-f]{64}$/i;
const COMMIT_PATTERN = /^(?:[0-9a-f]{40}|[0-9a-f]{64})$/i;

export type RepoLinkInput = {
  repoId: string;
  owner?: string | null;
  ref: string;
  path: string;
};

export type ParsedRepoLink = {
  projectId: string;
  repoId: string;
  owner: string | null;
  ref: string;
  path: string;
};

export type RepoLinkParseResult =
  | { ok: true; value: ParsedRepoLink }
  | { ok: false; reason: string };

export function isSafeRepoPath(path: string): boolean {
  if (!path || path.startsWith("/") || path.includes("\\")) return false;
  return path
    .split("/")
    .every((segment) => segment !== "" && segment !== "." && segment !== "..");
}

export function buildRepoLink(input: RepoLinkInput): string {
  if (!REPO_ID_PATTERN.test(input.repoId)) {
    throw new Error("buildRepoLink: invalid repoId");
  }
  if (input.owner && !HEX_64_PATTERN.test(input.owner)) {
    throw new Error("buildRepoLink: invalid owner");
  }
  if (!COMMIT_PATTERN.test(input.ref)) {
    throw new Error("buildRepoLink: ref must be a full commit hash");
  }
  if (!isSafeRepoPath(input.path)) {
    throw new Error("buildRepoLink: path must be a safe relative path");
  }

  const params = new URLSearchParams();
  params.set("repo", input.repoId);
  if (input.owner) params.set("owner", input.owner.toLowerCase());
  params.set("ref", input.ref.toLowerCase());
  params.set("path", input.path);
  return `${REPO_LINK_SCHEME}//${REPO_LINK_HOST}?${params.toString()}`;
}

export function parseRepoLink(url: string): RepoLinkParseResult {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return { ok: false, reason: "invalid-url" };
  }
  if (parsed.protocol !== REPO_LINK_SCHEME) {
    return { ok: false, reason: "wrong-scheme" };
  }
  if (parsed.hostname !== REPO_LINK_HOST) {
    return { ok: false, reason: "wrong-host" };
  }

  const repoId = parsed.searchParams.get("repo") ?? "";
  const owner = parsed.searchParams.get("owner");
  const ref = parsed.searchParams.get("ref") ?? "";
  const path = parsed.searchParams.get("path") ?? "";
  if (!REPO_ID_PATTERN.test(repoId))
    return { ok: false, reason: "invalid-repo" };
  if (owner !== null && !HEX_64_PATTERN.test(owner)) {
    return { ok: false, reason: "invalid-owner" };
  }
  if (!COMMIT_PATTERN.test(ref)) return { ok: false, reason: "invalid-ref" };
  if (!isSafeRepoPath(path)) return { ok: false, reason: "invalid-path" };

  const normalizedOwner = owner?.toLowerCase() ?? null;
  return {
    ok: true,
    value: {
      projectId: normalizedOwner ? `${normalizedOwner}:${repoId}` : repoId,
      repoId,
      owner: normalizedOwner,
      ref: ref.toLowerCase(),
      path,
    },
  };
}

export function isRepoLink(href: string | undefined | null): boolean {
  if (!href) return false;
  return href.startsWith("buzz://repo?") || href === "buzz://repo";
}
