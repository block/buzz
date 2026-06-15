import * as React from "react";
import { AlertTriangle, ChevronDown, RefreshCw } from "lucide-react";

import {
  useAgentMemoryGraph,
  useIsManagedAgent,
} from "@/features/agent-memory/hooks";
import type { MemoryTreeNode } from "@/features/agent-memory/lib/buildMemoryGraph";
import type { EngramEntry } from "@/shared/api/tauriEngrams";
import { cn } from "@/shared/lib/cn";
import { Button, type ButtonProps } from "@/shared/ui/button";
import { Skeleton } from "@/shared/ui/skeleton";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

const MEMORY_LIST_PREVIEW_LIMIT = 3;

const MEMORY_TRUNCATED_TOOLTIP =
  "This list may be incomplete — the relay returned the maximum number of memories.";

const MEMORY_DANGLING_REF_TOOLTIP =
  "This memory links to a slug that wasn't found in the loaded memory list.";

/**
 * Memory section — IXI-7 phase 1 read-only viewer.
 *
 * Owner-gated: returns `null` for non-owners (and while the managed-agent
 * list is still loading, to avoid a one-frame flash of the section before
 * we know the viewer's owner status).
 *
 * The whole thing is contained: skeleton/error/empty live *inside* this
 * section so the rest of the profile panel stays interactive while we
 * fetch+decrypt. Refetch is non-blocking (cached data stays visible while
 * `isFetching` is true).
 *
 * Layout:
 *   ⚠ truncated relay banner (if applicable)
 *   ── tree rooted at `core` ──
 *   ── orphans list (if any) ──
 *   ── missing [[slug]] refs highlighted inline on parent memories ──
 *
 * tho will refine the visual design — this is the structural placement.
 */
export function MemorySection({
  agentPubkey,
}: {
  agentPubkey: string;
}): React.ReactElement | null {
  const isOwner = useIsManagedAgent(agentPubkey);

  // Hide entirely for non-owners. Defer (`null`) while the managed-agent
  // list is still loading rather than render a placeholder we'll yank.
  if (isOwner !== true) return null;

  return <MemorySectionForOwner agentPubkey={agentPubkey} />;
}

export function MemoryRefreshButton({
  agentPubkey,
  className,
  iconClassName,
  variant = "ghost",
}: {
  agentPubkey: string;
  className?: string;
  iconClassName?: string;
  variant?: ButtonProps["variant"];
}): React.ReactElement | null {
  const isOwner = useIsManagedAgent(agentPubkey);
  const { query } = useAgentMemoryGraph(agentPubkey, {
    enabled: isOwner === true,
  });

  if (isOwner !== true || !query.data) return null;

  return (
    <Button
      aria-label="Refresh memory"
      className={cn(className, query.isFetching && "cursor-wait")}
      data-testid="agent-memory-refetch"
      disabled={query.isFetching}
      onClick={() => query.refetch()}
      size="icon"
      type="button"
      variant={variant}
    >
      <RefreshCw
        className={cn(
          iconClassName ?? "h-4 w-4",
          query.isFetching && "animate-spin",
        )}
      />
    </Button>
  );
}

function MemorySectionForOwner({ agentPubkey }: { agentPubkey: string }) {
  const { query, graph } = useAgentMemoryGraph(agentPubkey);

  // Order matters here. We want:
  // - first paint, no cache → skeleton
  // - error with no cache → error state (with retry)
  // - error WITH cache → keep the data, show a non-blocking "refetch failed"
  //   banner; user can retry without losing what they had
  // - data, but empty → empty state ("This agent has no memories yet")
  // - data, non-empty → render
  const showInitialSkeleton = query.isLoading && !query.data;
  const showInitialError = query.isError && !query.data;

  return (
    <section data-testid="agent-memory-section">
      {showInitialSkeleton ? <MemorySkeleton /> : null}

      {showInitialError ? (
        <MemoryErrorState
          error={query.error}
          onRetry={() => query.refetch()}
          retrying={query.isFetching}
        />
      ) : null}

      {query.data && graph ? (
        <>
          {/* Stale-cache error banner: shown when a refetch fails but we
              still have prior data on screen. Distinct from the initial
              error state above. */}
          {query.isError && !query.isFetching ? (
            <MemoryStaleErrorBanner onRetry={() => query.refetch()} />
          ) : null}

          <MemoryGraphView graph={graph} truncated={query.data.truncated} />
        </>
      ) : null}
    </section>
  );
}

// ── Subviews ────────────────────────────────────────────────────────────────

function MemorySkeleton() {
  return (
    <div
      aria-label="Loading memory"
      className="space-y-2"
      data-testid="agent-memory-skeleton"
      role="status"
    >
      <Skeleton className="h-4 w-2/3" />
      <Skeleton className="h-4 w-1/2" />
      <Skeleton className="h-4 w-3/5" />
    </div>
  );
}

function MemoryErrorState({
  error,
  onRetry,
  retrying,
}: {
  error: unknown;
  onRetry: () => void;
  retrying: boolean;
}) {
  const message =
    error instanceof Error ? error.message : String(error ?? "unknown error");
  return (
    <div
      className="flex flex-col gap-2 rounded-md border border-destructive/30 bg-destructive/5 p-3 text-xs"
      data-testid="agent-memory-error"
      role="alert"
    >
      <div className="flex items-start gap-2">
        <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-destructive" />
        <div className="space-y-1">
          <div className="font-medium text-destructive">
            Couldn't load memory
          </div>
          <div className="text-muted-foreground">{message}</div>
        </div>
      </div>
      <Button
        className="self-start"
        disabled={retrying}
        onClick={onRetry}
        size="sm"
        variant="outline"
      >
        {retrying ? "Retrying…" : "Retry"}
      </Button>
    </div>
  );
}

function MemoryStaleErrorBanner({ onRetry }: { onRetry: () => void }) {
  return (
    <div
      className="mb-2 flex items-center gap-2 rounded-md border border-warning/30 bg-warning/5 px-2 py-1.5 text-xs"
      data-testid="agent-memory-stale-error"
    >
      <AlertTriangle className="h-4 w-4 shrink-0 text-warning" />
      <span className="flex-1 text-muted-foreground">Refresh failed.</span>
      <button
        className="font-medium text-warning hover:underline"
        onClick={onRetry}
        type="button"
      >
        Retry
      </button>
    </div>
  );
}

function MemoryGraphView({
  graph,
  truncated,
}: {
  graph: NonNullable<ReturnType<typeof useAgentMemoryGraph>["graph"]>;
  truncated: boolean;
}) {
  const { rootedTree, orphans, dangling } = graph;
  const [showAllEntries, setShowAllEntries] = React.useState(false);
  const danglingSlugs = React.useMemo(
    () => new Set(dangling.map((d) => d.slug)),
    [dangling],
  );

  const isEmpty = !rootedTree && orphans.length === 0;
  if (isEmpty) {
    return (
      <p
        className="text-sm italic text-muted-foreground"
        data-testid="agent-memory-empty"
      >
        This agent has no memories yet.
      </p>
    );
  }

  const core = rootedTree?.entry ?? null;
  const memories = [
    ...(rootedTree ? flattenTreeDescendants(rootedTree) : []),
    ...orphans,
  ];
  const entries = [...(core ? [core] : []), ...memories];
  const hasMoreEntries = entries.length > MEMORY_LIST_PREVIEW_LIMIT;
  const visibleEntries = showAllEntries
    ? entries
    : entries.slice(0, MEMORY_LIST_PREVIEW_LIMIT);

  return (
    <div className="space-y-3">
      {!core && memories.length > 0 ? (
        <p
          className="text-xs italic text-muted-foreground"
          data-testid="agent-memory-no-core"
        >
          No <code className="font-mono text-[10px]">core</code> memory yet —
          agent identity is unrooted.
        </p>
      ) : null}

      <div className="space-y-2" data-testid="agent-memory-list">
        {visibleEntries.map((entry) => (
          <MemoryEntryAccordion
            danglingSlugs={danglingSlugs}
            entry={entry}
            key={entry.eventId}
          />
        ))}
      </div>

      {hasMoreEntries && !showAllEntries ? (
        <MemoryShowMoreButton
          count={entries.length}
          onClick={() => setShowAllEntries(true)}
          truncated={truncated}
        />
      ) : null}

      {truncated && !hasMoreEntries ? <MemoryTruncatedHint /> : null}

      {hasMoreEntries && showAllEntries ? (
        <button
          className="flex w-full justify-center rounded-2xl bg-muted/40 px-4 py-3 text-sm font-medium text-foreground transition-colors hover:bg-muted/50"
          data-testid="agent-memory-show-less"
          onClick={() => setShowAllEntries(false)}
          type="button"
        >
          Show less
        </button>
      ) : null}
    </div>
  );
}

function MemoryShowMoreButton({
  count,
  onClick,
  truncated,
}: {
  count: number;
  onClick: () => void;
  truncated: boolean;
}) {
  const button = (
    <button
      className="flex w-full items-center justify-center gap-2 rounded-2xl bg-muted/40 px-4 py-3 text-sm font-medium text-foreground transition-colors hover:bg-muted/50"
      data-testid={
        truncated ? "agent-memory-truncated" : "agent-memory-show-more"
      }
      onClick={onClick}
      type="button"
    >
      View all ({count})
    </button>
  );

  if (!truncated) return button;

  return (
    <Tooltip>
      <TooltipTrigger asChild>{button}</TooltipTrigger>
      <TooltipContent className="max-w-xs text-xs" side="top">
        {MEMORY_TRUNCATED_TOOLTIP}
      </TooltipContent>
    </Tooltip>
  );
}

function MemoryTruncatedHint() {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <div
          className="flex justify-center rounded-2xl border border-warning/30 bg-warning/5 px-4 py-2"
          data-testid="agent-memory-truncated"
        >
          <AlertTriangle className="h-4 w-4 text-warning" />
        </div>
      </TooltipTrigger>
      <TooltipContent className="max-w-xs text-xs" side="top">
        {MEMORY_TRUNCATED_TOOLTIP}
      </TooltipContent>
    </Tooltip>
  );
}

function flattenTreeDescendants(node: MemoryTreeNode): EngramEntry[] {
  const entries: EngramEntry[] = [];
  for (const child of node.children) {
    entries.push(child.entry);
    entries.push(...flattenTreeDescendants(child));
  }
  return entries;
}

const MEMORY_REF_PATTERN = /\[\[([^\]]+)\]\]/g;

function MemoryBodyText({ body }: { body: string }) {
  const parts: React.ReactNode[] = [];
  let lastIndex = 0;

  for (const match of body.matchAll(MEMORY_REF_PATTERN)) {
    const index = match.index ?? 0;
    const slug = match[1];
    if (index > lastIndex) {
      parts.push(body.slice(lastIndex, index));
    }
    parts.push(
      <span
        className="wrap-break-word break-all text-foreground"
        key={`${index}-${slug}`}
      >
        [[{slug}]]
      </span>,
    );
    lastIndex = index + match[0].length;
  }

  if (lastIndex < body.length) {
    parts.push(body.slice(lastIndex));
  }

  return <>{parts}</>;
}

function MemoryDanglingRefsHint({ slugs }: { slugs: string[] }) {
  if (slugs.length === 0) return null;

  return (
    <div
      className="-mx-4 -mb-2 mt-2 px-2 pb-1"
      data-testid="agent-memory-dangling-hint"
    >
      <div className="rounded-xl bg-warning/5 px-2.5 py-2 text-xs leading-5">
        <p className="text-warning">
          Missing {slugs.length === 1 ? "link" : "links"}:{" "}
          {slugs.map((slug, index) => (
            <React.Fragment key={slug}>
              {index > 0 ? ", " : null}
              <span
                className="wrap-break-word break-all text-warning"
                data-testid="agent-memory-dangling-ref"
              >
                [[{slug}]]
              </span>
            </React.Fragment>
          ))}
        </p>
        <p className="mt-0.5 text-foreground/50">
          {MEMORY_DANGLING_REF_TOOLTIP}
        </p>
      </div>
    </div>
  );
}

function MemorySlugTitle({ slug }: { slug: string }) {
  const segments = slug.split("/").filter((segment) => segment.length > 0);
  if (segments.length === 0) return null;

  if (segments.length === 1) {
    return (
      <span
        className={segments[0] === "mem" ? "text-foreground/40" : undefined}
      >
        {segments[0]}
      </span>
    );
  }

  return (
    <span className="inline-flex flex-wrap items-baseline">
      {segments.map((segment, index) => {
        const segmentPath = segments.slice(0, index + 1).join("/");
        return (
          <React.Fragment key={segmentPath}>
            {index > 0 ? (
              <span className="px-0.5 text-foreground/40">/</span>
            ) : null}
            <span
              className={cn(
                segment === "mem" ? "text-foreground/40" : "text-foreground",
              )}
            >
              {segment}
            </span>
          </React.Fragment>
        );
      })}
    </span>
  );
}

function elementExceedsLines(element: HTMLElement, lines: number): boolean {
  const lineHeight = Number.parseFloat(getComputedStyle(element).lineHeight);
  if (!Number.isFinite(lineHeight) || lineHeight <= 0) {
    return element.scrollHeight > element.clientHeight + 1;
  }
  return element.scrollHeight > lineHeight * lines + 1;
}

/** A single engram accordion — collapsed preview truncates to two lines. */
function MemoryEntryAccordion({
  danglingSlugs,
  entry,
}: {
  danglingSlugs: ReadonlySet<string>;
  entry: EngramEntry;
}) {
  const [open, setOpen] = React.useState(false);
  const [showCaret, setShowCaret] = React.useState(false);
  const articleRef = React.useRef<HTMLElement>(null);
  const titleRef = React.useRef<HTMLDivElement>(null);
  const bodyRef = React.useRef<HTMLDivElement>(null);
  const isEmpty = entry.body.trim().length === 0;
  const danglingRefsForEntry = entry.outgoingRefs.filter((ref) =>
    danglingSlugs.has(ref),
  );
  const hasDanglingRefs = danglingRefsForEntry.length > 0;
  const canExpand = showCaret || hasDanglingRefs;

  // biome-ignore lint/correctness/useExhaustiveDependencies: remeasure when accordion clamping changes
  React.useLayoutEffect(() => {
    const measure = () => {
      const titleEl = titleRef.current;
      const bodyEl = bodyRef.current;
      if (!titleEl || !bodyEl) return;

      setShowCaret(
        elementExceedsLines(titleEl, 2) || elementExceedsLines(bodyEl, 2),
      );
    };

    measure();
    const root = articleRef.current;
    if (!root) return undefined;

    const observer = new ResizeObserver(measure);
    observer.observe(root);
    return () => observer.disconnect();
  }, [open]);

  const content = (
    <>
      <div className="min-w-0 flex-1">
        <div
          className={cn(
            "text-sm font-semibold text-foreground",
            !open && "line-clamp-2",
          )}
          ref={titleRef}
        >
          {hasDanglingRefs ? (
            <AlertTriangle className="mr-1 inline-block h-4 w-4 align-[-2px] text-warning" />
          ) : null}
          <MemorySlugTitle slug={entry.slug} />
        </div>
        <div
          className={cn(
            "mt-1 text-xs leading-5 text-foreground/70",
            open ? "whitespace-pre-wrap wrap-break-word" : "line-clamp-2",
          )}
          ref={bodyRef}
        >
          {isEmpty ? (
            <span className="italic text-foreground/50">(empty)</span>
          ) : (
            <MemoryBodyText body={entry.body} />
          )}
        </div>
      </div>
      {canExpand ? (
        <ChevronDown
          className={cn(
            "mt-0.5 h-4 w-4 shrink-0 text-muted-foreground transition-transform",
            open && "rotate-180",
          )}
        />
      ) : null}
    </>
  );

  return (
    <article
      className="overflow-hidden rounded-2xl bg-muted/40"
      ref={articleRef}
    >
      {canExpand ? (
        <button
          aria-expanded={open}
          className="w-full px-4 py-3 text-left transition-colors hover:bg-muted/50"
          onClick={() => setOpen((value) => !value)}
          type="button"
        >
          <div className="flex items-start gap-3">{content}</div>
          {hasDanglingRefs && open ? (
            <MemoryDanglingRefsHint slugs={danglingRefsForEntry} />
          ) : null}
        </button>
      ) : (
        <div className="flex items-start gap-3 px-4 py-3">{content}</div>
      )}
    </article>
  );
}
