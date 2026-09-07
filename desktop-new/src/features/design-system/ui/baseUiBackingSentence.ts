import { type BaseUiPart, resolveBaseUiBacking } from "@/shared/ui/registry";

/**
 * The sentence under a component's title, as segments rather than as markup.
 *
 * Split out from the component so the phrasing is testable without a DOM: the
 * component renders these segments and nothing else, so a test asserting the
 * flattened text is asserting what the page actually says. Building the string
 * and the markup separately would let the two drift, which is the failure this
 * shape exists to prevent.
 */
export type SentenceSegment =
  | { kind: "text"; value: string }
  | { kind: "part"; part: BaseUiPart };

function text(value: string): SentenceSegment {
  return { kind: "text", value };
}

function part(value: BaseUiPart): SentenceSegment {
  return { kind: "part", part: value };
}

/** "Field and Input", "Field, Input and Button" — a readable list, not commas. */
function partList(parts: readonly BaseUiPart[]): SentenceSegment[] {
  return parts.flatMap((value, index) => {
    if (index === 0) return [part(value)];
    const separator = index === parts.length - 1 ? " and " : ", ";
    return [text(separator), part(value)];
  });
}

/**
 * Four answers, because there are four genuinely different situations:
 *
 *   own only        Built on Base UI Button.
 *   inherited only  No Base UI part of its own. Inherits Base UI Button
 *                   through Buzz Button.
 *   both            Built on Base UI Field and Input. Also inherits Base UI
 *                   Button through Buzz Button.
 *   neither         No Base UI part. Semantic native header.
 *
 * The last says what the component *is* rather than trailing off after the
 * dash — authoring its own markup is the decision, not a gap, and a sentence
 * that only reports an absence reads like something is missing.
 */
export function baseUiBackingSentence(
  slug: string,
  behavior: string,
): SentenceSegment[] {
  const { own, inherited } = resolveBaseUiBacking(slug);

  if (own.length === 0 && inherited.length === 0) {
    return [text(`No Base UI part. ${behavior}.`)];
  }

  const segments: SentenceSegment[] =
    own.length > 0
      ? [text("Built on Base UI "), ...partList(own), text(".")]
      : [text("No Base UI part of its own.")];

  for (const entry of inherited) {
    segments.push(
      text(own.length > 0 ? " Also inherits Base UI " : " Inherits Base UI "),
      part(entry.part),
      text(` through Buzz ${entry.through}.`),
    );
  }

  return segments;
}

/** The sentence as plain text — what a reader sees, links flattened. */
export function flattenSentence(segments: readonly SentenceSegment[]): string {
  return segments
    .map((segment) =>
      segment.kind === "text" ? segment.value : segment.part.name,
    )
    .join("");
}
