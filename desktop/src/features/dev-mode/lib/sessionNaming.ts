const MAX_SLUG_LENGTH = 40;

/** Reduce arbitrary text to channel-name form: lowercase kebab-case. */
export function sanitizeChannelName(text: string): string {
  return text
    .toLowerCase()
    .replace(/[^a-z0-9\s-]/g, "")
    .trim()
    .split(/\s+/)
    .reduce((slug, word) => {
      if (word.length === 0) return slug;
      if (slug.length === 0) return word.slice(0, MAX_SLUG_LENGTH);
      const next = `${slug}-${word}`;
      return next.length > MAX_SLUG_LENGTH ? slug : next;
    }, "");
}

/** Suffix a base name until it does not collide with an existing channel. */
export function uniqueChannelName(
  base: string,
  existingNames: ReadonlySet<string>,
): string {
  if (!existingNames.has(base)) return base;
  for (let suffix = 2; ; suffix += 1) {
    const candidate = `${base}-${suffix}`;
    if (!existingNames.has(candidate)) return candidate;
  }
}

/** Derive a channel name from the first words of a prompt. */
export function slugifyPrompt(
  prompt: string,
  existingNames: ReadonlySet<string>,
): string {
  return uniqueChannelName(
    sanitizeChannelName(prompt) || "session",
    existingNames,
  );
}
