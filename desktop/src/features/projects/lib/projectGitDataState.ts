export type ProjectGitDataState =
  | "checking"
  | "available"
  | "empty"
  | "unavailable";

/**
 * Whether the Code tab can show a tree/README for the selected source.
 *
 * GitHub remotes are first-class for *reading* (public clone). They used to
 * force `unavailable` so the UI asked people to "clone locally" even when
 * Buzz can snapshot the same URL. Local checkout is still the edit surface.
 */
export function projectGitDataState({
  error,
  fileCount,
  hasSnapshot,
  loading,
}: {
  error: unknown;
  fileCount: number;
  hasSnapshot: boolean;
  loading: boolean;
}): ProjectGitDataState {
  if (loading) return "checking";
  if (error || !hasSnapshot) return "unavailable";
  if (fileCount === 0) return "empty";
  return "available";
}
