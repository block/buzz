/** Case-insensitive token matching for Projects-local search. */
export function matchesProjectsSearch(
  query: string,
  values: ReadonlyArray<string | null | undefined>,
) {
  const tokens = query.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
  if (tokens.length === 0) return true;
  const haystack = values.filter(Boolean).join(" ").toLocaleLowerCase();
  return tokens.every((token) => haystack.includes(token));
}
