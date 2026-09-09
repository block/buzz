import {
  baseUiDocsUrl,
  COMPONENTS,
  resolveBaseUiBacking,
} from "@/shared/ui/registry";

export type SentenceSegment =
  | { kind: "text"; value: string }
  | { kind: "link"; value: string; href: string; external: boolean };

/** Direct Base UI imports link upstream; inherited backing links to its Buzz owner. */
export function baseUiBackingSentence(slug: string): SentenceSegment[] {
  const { own, inherited } = resolveBaseUiBacking(slug);
  const links: Extract<SentenceSegment, { kind: "link" }>[] = own.map(
    (part) => ({
      kind: "link",
      value: `Base UI ${part.name}`,
      href: baseUiDocsUrl(part),
      external: true,
    }),
  );
  const seen = new Set<string>();
  for (const entry of inherited) {
    const component = COMPONENTS.find(
      (candidate) => candidate.name === entry.through,
    );
    if (!component || seen.has(component.slug)) continue;
    seen.add(component.slug);
    links.push({
      kind: "link",
      value: component.name,
      href: `/design/components/${component.slug}`,
      external: false,
    });
  }
  if (!links.length) return [];
  const segments: SentenceSegment[] = [{ kind: "text", value: "Inherits " }];
  links.forEach((link, index) => {
    if (index)
      segments.push({
        kind: "text",
        value: index === links.length - 1 ? " and " : ", ",
      });
    segments.push(link);
  });
  segments.push({ kind: "text", value: "." });
  return segments;
}

/** Plain text of the same segments used by the rendered description. */
export function flattenSentence(segments: readonly SentenceSegment[]): string {
  return segments.map((segment) => segment.value).join("");
}
