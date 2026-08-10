import * as React from "react";
import { ChevronDown, ChevronRight, Link2, Link2Off } from "lucide-react";

import type { Backlinks, Mention } from "@/features/documents/lib/backlinks";
import { groupMentionsBySource } from "@/features/documents/lib/backlinks";

function MentionGroup({
  mentions,
  onOpen,
  sourceName,
  sourcePath,
}: {
  mentions: Mention[];
  onOpen: (path: string) => void;
  sourceName: string;
  sourcePath: string;
}) {
  return (
    <li className="px-3 py-1.5">
      <button
        className="w-full truncate text-left text-sm font-medium hover:underline"
        onClick={() => onOpen(sourcePath)}
        title={sourcePath}
        type="button"
      >
        {sourceName}
      </button>
      <ul className="mt-1 space-y-1">
        {mentions.map((mention) => (
          <li key={`${mention.sourcePath}:${mention.lineNumber}`}>
            <button
              className="w-full text-left text-2xs text-muted-foreground hover:text-foreground"
              onClick={() => onOpen(mention.sourcePath)}
              type="button"
            >
              <span className="line-clamp-2">{mention.line.trim()}</span>
            </button>
          </li>
        ))}
      </ul>
    </li>
  );
}

function Section({
  emptyLabel,
  icon,
  mentions,
  onOpen,
  testId,
  title,
}: {
  emptyLabel: string;
  icon: React.ReactNode;
  mentions: Mention[];
  onOpen: (path: string) => void;
  testId: string;
  title: string;
}) {
  const groups = groupMentionsBySource(mentions);
  const [collapsed, setCollapsed] = React.useState(false);
  const Chevron = collapsed ? ChevronRight : ChevronDown;

  return (
    <section data-testid={testId}>
      <button
        aria-expanded={!collapsed}
        className="flex w-full items-center gap-1.5 px-3 py-2 text-2xs font-medium uppercase tracking-wide text-muted-foreground hover:text-foreground"
        data-testid={`${testId}-toggle`}
        onClick={() => setCollapsed((current) => !current)}
        type="button"
      >
        <Chevron className="h-3 w-3 shrink-0" />
        {icon}
        {title}
        <span className="ml-auto tabular-nums">{mentions.length}</span>
      </button>
      {collapsed ? null : groups.length === 0 ? (
        <p className="px-3 pb-2 text-2xs text-muted-foreground">{emptyLabel}</p>
      ) : (
        <ul>
          {groups.map((group) => (
            <MentionGroup
              key={group.sourcePath}
              mentions={group.mentions}
              onOpen={onOpen}
              sourceName={group.sourceName}
              sourcePath={group.sourcePath}
            />
          ))}
        </ul>
      )}
    </section>
  );
}

/**
 * Linked and unlinked mentions of the open note.
 *
 * Unlinked mentions are kept in their own section rather than mixed in: they
 * are a suggestion ("you named this note but did not link it"), not a fact
 * about the graph.
 */
export function DocumentBacklinksPanel({
  backlinks,
  onOpen,
}: {
  backlinks: Backlinks;
  onOpen: (path: string) => void;
}) {
  return (
    <div
      className="min-h-0 flex-1 overflow-y-auto"
      data-testid="documents-backlinks"
    >
      <Section
        emptyLabel="No notes link here yet."
        icon={<Link2 className="h-3.5 w-3.5" />}
        mentions={backlinks.linked}
        onOpen={onOpen}
        testId="documents-linked-mentions"
        title="Linked mentions"
      />
      <Section
        emptyLabel="No unlinked mentions."
        icon={<Link2Off className="h-3.5 w-3.5" />}
        mentions={backlinks.unlinked}
        onOpen={onOpen}
        testId="documents-unlinked-mentions"
        title="Unlinked mentions"
      />
    </div>
  );
}
