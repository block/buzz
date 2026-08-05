import type { OutlineHeading } from "@/features/documents/lib/obsidianSyntax";
import { cn } from "@/shared/lib/cn";

/**
 * Heading outline for the open note.
 *
 * Indentation is capped at three levels of nesting: an `h5` under an `h4` in a
 * 260px rail would otherwise have almost no room left for its text.
 */
export function DocumentOutlinePanel({
  activeIndex,
  headings,
  onSelect,
}: {
  activeIndex: number;
  headings: readonly OutlineHeading[];
  onSelect: (heading: OutlineHeading) => void;
}) {
  return (
    <section data-testid="documents-outline">
      <h3 className="px-3 py-2 text-2xs font-medium uppercase tracking-wide text-muted-foreground">
        Outline
      </h3>
      {headings.length === 0 ? (
        <p className="px-3 pb-2 text-2xs text-muted-foreground">
          No headings in this note.
        </p>
      ) : (
        <ul className="pb-2">
          {headings.map((heading, index) => (
            <li key={`${heading.position}:${heading.text}`}>
              <button
                className={cn(
                  "w-full truncate py-1 pr-3 text-left text-xs",
                  index === activeIndex
                    ? "text-foreground"
                    : "text-muted-foreground hover:text-foreground",
                )}
                data-testid={`documents-outline-item-${heading.text}`}
                onClick={() => onSelect(heading)}
                style={{
                  paddingLeft: `${12 + Math.min(heading.level - 1, 3) * 10}px`,
                }}
                title={heading.text}
                type="button"
              >
                {heading.text}
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
