import * as React from "react";
import { ChevronDown, ChevronRight } from "lucide-react";

import type { OutlineHeading } from "@/features/documents/lib/obsidianSyntax";
import { cn } from "@/shared/lib/cn";

const COLLAPSED_KEY = "buzz.documents.outline-collapsed";

/** Remembers the collapsed state across sessions, as Onyx's panels do. */
function useCollapsed(storageKey: string) {
  const [collapsed, setCollapsed] = React.useState(() => {
    try {
      return window.localStorage.getItem(storageKey) === "1";
    } catch {
      return false;
    }
  });

  const toggle = React.useCallback(() => {
    setCollapsed((current) => {
      const next = !current;
      try {
        window.localStorage.setItem(storageKey, next ? "1" : "0");
      } catch {
        // Losing the preference is not worth failing the interaction.
      }
      return next;
    });
  }, [storageKey]);

  return [collapsed, toggle] as const;
}

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
  const [collapsed, toggle] = useCollapsed(COLLAPSED_KEY);
  const Chevron = collapsed ? ChevronRight : ChevronDown;

  return (
    <section data-testid="documents-outline">
      <button
        aria-expanded={!collapsed}
        className="flex w-full items-center gap-1 px-3 py-2 text-2xs font-medium uppercase tracking-wide text-muted-foreground hover:text-foreground"
        data-testid="documents-outline-toggle"
        onClick={toggle}
        type="button"
      >
        <Chevron className="h-3 w-3 shrink-0" />
        Outline
        <span className="ml-auto tabular-nums">{headings.length}</span>
      </button>
      {collapsed ? null : headings.length === 0 ? (
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
