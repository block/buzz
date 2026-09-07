import { baseUiDocsUrl } from "@/shared/ui/registry";

import { baseUiBackingSentence } from "./baseUiBackingSentence";

/**
 * One sentence saying what a component is built on, shown under its title.
 *
 * The phrasing lives in `baseUiBackingSentence` so it is testable without a DOM
 * and so every component page says it the same way; this component only renders
 * those segments, turning each Base UI part into a link to its documentation.
 *
 * What it says comes from the registry, which `src/shared/ui/registry.test.ts`
 * binds to the imports in the component files — a component that gains or drops
 * a Base UI part fails that test rather than leaving a stale sentence here.
 */
export function BaseUiBackingLine({
  slug,
  behavior,
}: {
  slug: string;
  /** What the component does instead, for the case with no Base UI part at all. */
  behavior: string;
}) {
  const segments = baseUiBackingSentence(slug, behavior);

  return (
    <p className="text-body text-secondary">
      {segments.map((segment, index) =>
        segment.kind === "text" ? (
          // biome-ignore lint/suspicious/noArrayIndexKey: a text run has no id but its position is stable
          <span key={index}>{segment.value}</span>
        ) : (
          <a
            key={segment.part.name}
            href={baseUiDocsUrl(segment.part)}
            target="_blank"
            rel="noreferrer"
            className="text-purple-12 underline decoration-1 underline-offset-2"
          >
            {segment.part.name}
          </a>
        ),
      )}
    </p>
  );
}
