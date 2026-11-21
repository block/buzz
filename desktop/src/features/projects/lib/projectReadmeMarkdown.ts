const NAMED_HTML_ENTITIES: Record<string, string> = {
  amp: "&",
  apos: "'",
  gt: ">",
  lt: "<",
  quot: '"',
};

/** Decode exactly one HTML-entity layer so replacements are never re-decoded. */
function decodeHtmlEntitiesOnce(value: string): string {
  return value.replace(
    /&(?:#(\d+)|#x([\da-f]+)|([a-z]+));/gi,
    (
      match,
      decimal: string | undefined,
      hex: string | undefined,
      named: string | undefined,
    ) => {
      if (named) {
        return NAMED_HTML_ENTITIES[named.toLowerCase()] ?? match;
      }

      const codePoint = Number.parseInt(
        decimal ?? hex ?? "",
        decimal ? 10 : 16,
      );
      if (
        !Number.isInteger(codePoint) ||
        codePoint < 0 ||
        codePoint > 0x10ffff ||
        (codePoint >= 0xd800 && codePoint <= 0xdfff)
      ) {
        return match;
      }
      return String.fromCodePoint(codePoint);
    },
  );
}

function markdownDestination(value: string, image: boolean): string | null {
  const trimmed = value.trim();
  const containsControlCharacter = Array.from(trimmed).some((character) => {
    const codePoint = character.codePointAt(0) ?? 0;
    return codePoint <= 0x1f || codePoint === 0x7f;
  });
  if (!trimmed || containsControlCharacter) {
    return null;
  }

  const scheme = /^([a-z][a-z\d+.-]*):/i.exec(trimmed)?.[1]?.toLowerCase();
  const allowedSchemes = image
    ? new Set(["http", "https"])
    : new Set(["buzz", "http", "https", "mailto"]);
  if (scheme && !allowedSchemes.has(scheme)) {
    return null;
  }

  return trimmed
    .replace(/\\/g, "%5C")
    .replace(/ /g, "%20")
    .replace(/\(/g, "%28")
    .replace(/\)/g, "%29");
}

function escapeMarkdownLabel(value: string): string {
  return value
    .replace(/\\/g, "\\\\")
    .replace(/\[/g, "\\[")
    .replace(/\]/g, "\\]");
}

/**
 * Convert the small inline-HTML subset commonly used by README files.
 * Anything outside the allowlist is rendered as inert text, never removed by
 * a regex that could expose a second tag after replacement.
 */
function convertDecodedInlineHtml(value: string): string {
  return value
    .replace(/<br\s*\/?\s*>/gi, "\n")
    .replace(/<img\b([^>]*)>/gi, (_match: string, attrs: string) => {
      const rawSource = attrs.match(/\bsrc=["']([^"']+)["']/i)?.[1];
      const source = rawSource ? markdownDestination(rawSource, true) : null;
      const alt = escapeMarkdownLabel(
        attrs.match(/\balt=["']([^"']*)["']/i)?.[1] ?? "",
      );
      return source ? `![${alt}](${source})` : "";
    })
    .replace(
      /<a\b[^>]*\bhref=["']([^"']+)["'][^>]*>([\s\S]*?)<\/a>/gi,
      (_match: string, rawHref: string, label: string) => {
        const renderedLabel = escapeMarkdownLabel(
          convertDecodedInlineHtml(label).trim(),
        );
        const href = markdownDestination(rawHref, false);
        return href ? `[${renderedLabel}](${href})` : renderedLabel;
      },
    )
    .replace(/<(strong|b)\b[^>]*>([\s\S]*?)<\/\1>/gi, "**$2**")
    .replace(/<(em|i)\b[^>]*>([\s\S]*?)<\/\1>/gi, "*$2*")
    .replace(/<code\b[^>]*>([\s\S]*?)<\/code>/gi, "`$1`")
    .replace(/<sub\b[^>]*>([\s\S]*?)<\/sub>/gi, "$1")
    .replace(/<span\b[^>]*>([\s\S]*?)<\/span>/gi, "$1")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

export function normalizeReadmeMarkdown(content: string): string {
  const decoded = decodeHtmlEntitiesOnce(content);
  const blockNormalized = decoded
    .replace(
      /<h([1-6])\b[^>]*>([\s\S]*?)<\/h\1>/gi,
      (_match, depth: string, value: string) =>
        `${"#".repeat(Number(depth))} ${value}\n\n`,
    )
    .replace(/<p\b[^>]*>([\s\S]*?)<\/p>/gi, "$1\n\n")
    .replace(/<div\b[^>]*>([\s\S]*?)<\/div>/gi, "$1\n\n")
    .replace(/<center\b[^>]*>([\s\S]*?)<\/center>/gi, "$1\n\n");

  return convertDecodedInlineHtml(blockNormalized)
    .replace(/\n{3,}/g, "\n\n")
    .trim();
}
