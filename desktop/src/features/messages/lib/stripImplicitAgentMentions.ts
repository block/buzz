/**
 * Removes the leading mention prefix synthesized by automatic agent addressing.
 * Only the contiguous composer prefix is considered; identical mentions later in
 * the draft remain authored content.
 */
export function stripImplicitAgentMentions(
  content: string,
  displayNames: readonly string[],
): string {
  let stripped = content;
  const names = [...new Set(displayNames.map((name) => name.trim()))]
    .filter(Boolean)
    .sort((left, right) => right.length - left.length);

  while (stripped.startsWith("@")) {
    const name = names.find(
      (candidate) =>
        stripped === `@${candidate}` || stripped.startsWith(`@${candidate} `),
    );
    if (!name) break;
    stripped = stripped.slice(name.length + (stripped === `@${name}` ? 1 : 2));
  }

  return stripped;
}
