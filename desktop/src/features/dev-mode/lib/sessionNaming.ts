const MAX_SLUG_LENGTH = 40;

/** Derive a channel name from the first words of a prompt. */
export function slugifyPrompt(
  prompt: string,
  existingNames: ReadonlySet<string>,
): string {
  const base =
    prompt
      .toLowerCase()
      .replace(/[^a-z0-9\s-]/g, "")
      .trim()
      .split(/\s+/)
      .reduce((slug, word) => {
        if (slug.length === 0) return word.slice(0, MAX_SLUG_LENGTH);
        const next = `${slug}-${word}`;
        return next.length > MAX_SLUG_LENGTH ? slug : next;
      }, "") || "session";

  if (!existingNames.has(base)) return base;
  for (let suffix = 2; ; suffix += 1) {
    const candidate = `${base}-${suffix}`;
    if (!existingNames.has(candidate)) return candidate;
  }
}
