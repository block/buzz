import type { Project } from "@/features/projects/hooks";

function localRepoNameCandidate(value: string | null | undefined) {
  const trimmed = value?.trim().replace(/\.git$/i, "") ?? "";
  if (
    !trimmed ||
    trimmed === "." ||
    trimmed === ".." ||
    trimmed.includes("/") ||
    trimmed.includes("\\")
  ) {
    return null;
  }
  return trimmed;
}

function cloneUrlRepoName(cloneUrl: string | undefined) {
  if (!cloneUrl) return null;
  try {
    const parsed = new URL(cloneUrl);
    const segments = parsed.pathname.split("/").filter(Boolean);
    const lastSegment = segments[segments.length - 1];
    return localRepoNameCandidate(lastSegment);
  } catch {
    return null;
  }
}

function localRepoCandidates(project: Project) {
  return [
    localRepoNameCandidate(project.dtag),
    cloneUrlRepoName(project.cloneUrls[0]),
  ].filter((candidate, index, candidates): candidate is string =>
    Boolean(candidate && candidates.indexOf(candidate) === index),
  );
}

export function hasLocalCheckout(
  project: Project,
  localRepoNames: Set<string>,
) {
  return localRepoCandidates(project).some((candidate) =>
    localRepoNames.has(candidate),
  );
}
