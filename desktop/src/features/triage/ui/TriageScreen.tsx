import * as React from "react";
import { RefreshCw } from "lucide-react";
import { toast } from "sonner";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import type {
  TriageFeedbackAction,
  TriageSuggestion,
  TriageTodo,
} from "@/features/triage/api";
import {
  useAdoptTodoMutation,
  useResolveTodoMutation,
  useTriageFeedbackMutation,
  useTriageScanMutation,
  useTriageSuggestionsQuery,
  useTriageTodosQuery,
} from "@/features/triage/hooks";
import { useTriageAutoScan } from "@/features/triage/useTriageAutoScan";
import { useTriageCandidates } from "@/features/triage/useTriageCandidates";
import { TriageDetailPane } from "@/features/triage/ui/TriageDetailPane";
import { TriageListPane } from "@/features/triage/ui/TriageListPane";
import {
  TriageTabBar,
  type TriageTab,
} from "@/features/triage/ui/TriageTabBar";
import { TriageTodoList } from "@/features/triage/ui/TriageTodoList";
import { Button } from "@/shared/ui/button";

export function TriageScreen() {
  const { goChannel } = useAppNavigation();
  const { collect, currentPubkey, inboxItems, isLoading } =
    useTriageCandidates();

  const suggestionsQuery = useTriageSuggestionsQuery(currentPubkey);
  const todosQuery = useTriageTodosQuery(currentPubkey);
  const scanMutation = useTriageScanMutation(currentPubkey);
  const adoptMutation = useAdoptTodoMutation(currentPubkey);
  const resolveMutation = useResolveTodoMutation(currentPubkey);
  const feedbackMutation = useTriageFeedbackMutation(currentPubkey);

  const [activeTab, setActiveTab] = React.useState<TriageTab>("important");
  const [selectedEventId, setSelectedEventId] = React.useState<string | null>(
    null,
  );

  const suggestions = suggestionsQuery.data ?? [];

  // Partitioned straight from the stored verdict. Decisions are persisted by
  // the service rather than shadowed here, so they survive leaving the view.
  const { important, filtered } = React.useMemo(() => {
    const importantItems: TriageSuggestion[] = [];
    const filteredItems: TriageSuggestion[] = [];

    for (const suggestion of suggestions) {
      if (suggestion.adopted) continue;
      const target =
        suggestion.verdict === "attention" ? importantItems : filteredItems;
      target.push(suggestion);
    }

    return { important: importantItems, filtered: filteredItems };
  }, [suggestions]);

  const visible = activeTab === "filtered" ? filtered : important;
  const selected =
    visible.find((entry) => entry.eventId === selectedEventId) ?? null;

  const sendFeedback = React.useCallback(
    (suggestion: TriageSuggestion, userAction: TriageFeedbackAction) => {
      if (!currentPubkey) return;
      feedbackMutation.mutate({
        pubkey: currentPubkey,
        eventId: suggestion.eventId,
        channelId: suggestion.channelId,
        authorPubkey: suggestion.authorPubkey,
        threadRootId: suggestion.threadRootId,
        suggestedVerdict: suggestion.verdict,
        userAction,
        preview: suggestion.content.slice(0, 200),
      });
    },
    [currentPubkey, feedbackMutation],
  );

  const handleScan = React.useCallback(
    async ({ auto = false }: { auto?: boolean } = {}) => {
      try {
        const collection = await collect();
        const scanned = await scanMutation.mutateAsync(collection.candidates);

        if (auto) {
          // Automatic scans stay quiet unless they actually surfaced something.
          const attention = scanned.filter(
            (entry) => entry.verdict === "attention",
          ).length;
          if (attention > 0) {
            toast.success(
              `${attention} item${attention === 1 ? "" : "s"} need your attention`,
            );
          }
          return;
        }

        toast.success(
          `Triaged ${collection.candidates.length} unread messages ` +
            `(${collection.inboxCount} from inbox, ${collection.channelCount} from channels)`,
        );
      } catch (error) {
        // An auto-scan failure surfaces in the banner instead of a toast, so a
        // service that is down cannot spam notifications.
        if (auto) return;
        toast.error(
          error instanceof Error ? error.message : "Triage scan failed",
        );
      }
    },
    [collect, scanMutation],
  );

  const openThread = React.useCallback(
    (input: {
      channelId: string | null;
      eventId: string;
      threadRootId: string | null;
    }) => {
      if (!input.channelId) {
        toast.error("This message has no channel to open.");
        return;
      }
      void goChannel(input.channelId, {
        messageId: input.eventId,
        threadRootId: input.threadRootId,
      });
    },
    [goChannel],
  );

  const handleAdopt = React.useCallback(
    async (suggestion: TriageSuggestion) => {
      try {
        await adoptMutation.mutateAsync({
          eventId: suggestion.eventId,
          channelId: suggestion.channelId,
          channelName: suggestion.channelName,
          threadRootId: suggestion.threadRootId,
          authorLabel: suggestion.authorLabel,
          preview: suggestion.content.slice(0, 280),
          reason: suggestion.reason,
        });
        sendFeedback(suggestion, "adopted");
        toast.success("Added to your todos");
      } catch (error) {
        toast.error(
          error instanceof Error ? error.message : "Could not add the todo",
        );
      }
    },
    [adoptMutation, sendFeedback],
  );

  const handleDismiss = React.useCallback(
    (suggestion: TriageSuggestion) => {
      sendFeedback(suggestion, "dismissed");
      toast.success("Moved to Filtered — similar items will rank lower");
    },
    [sendFeedback],
  );

  const handlePromote = React.useCallback(
    (suggestion: TriageSuggestion) => {
      sendFeedback(suggestion, "promoted");
      setActiveTab("important");
      toast.success("Moved to Important — the agent will remember");
    },
    [sendFeedback],
  );

  const handleTodoResolve = React.useCallback(
    (todo: TriageTodo, status: "done" | "dismissed") => {
      void resolveMutation.mutateAsync({ id: todo.id, status });
      if (currentPubkey) {
        feedbackMutation.mutate({
          pubkey: currentPubkey,
          eventId: todo.eventId,
          channelId: todo.channelId,
          threadRootId: todo.threadRootId,
          suggestedVerdict: "attention",
          userAction: status === "done" ? "completed" : "dismissed",
          preview: todo.preview,
        });
      }
    },
    [currentPubkey, feedbackMutation, resolveMutation],
  );

  // Inbox arrivals the current results do not cover yet. Channel catch-up is
  // deliberately excluded: polling every channel to decide whether to scan
  // would cost the same relay queries the scan itself performs.
  const pendingCount = React.useMemo(() => {
    const known = new Set(suggestions.map((entry) => entry.eventId));
    return inboxItems.filter((entry) => !known.has(entry.item.id)).length;
  }, [inboxItems, suggestions]);

  useTriageAutoScan({
    enabled: Boolean(currentPubkey) && !suggestionsQuery.isLoading,
    isScanning: scanMutation.isPending,
    onScan: () => void handleScan({ auto: true }),
    pendingCount,
  });

  const listIsLoading = suggestionsQuery.isLoading || scanMutation.isPending;
  const scanError = suggestionsQuery.error ?? scanMutation.error;

  return (
    <div
      className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden"
      data-testid="triage-screen"
    >
      <div className="flex shrink-0 items-center gap-2 px-4 pt-4 sm:px-6">
        <TriageTabBar
          activeTab={activeTab}
          filteredCount={filtered.length}
          importantCount={important.length}
          onTabChange={setActiveTab}
          todoCount={
            todosQuery.data?.filter((todo) => todo.status === "open").length ??
            0
          }
        />
        <Button
          className="ml-auto"
          data-testid="triage-scan"
          disabled={scanMutation.isPending || isLoading}
          onClick={() => void handleScan({ auto: false })}
          size="sm"
          type="button"
          variant="outline"
        >
          <RefreshCw className="h-4 w-4" />
          {scanMutation.isPending ? "Scanning…" : "Scan now"}
        </Button>
      </div>

      {scanError ? (
        <p className="shrink-0 px-4 pt-3 text-sm text-destructive sm:px-6">
          {scanError instanceof Error
            ? scanError.message
            : "Could not load triage results"}
        </p>
      ) : null}

      {activeTab === "todos" ? (
        <div className="mt-3 flex min-h-0 flex-1 flex-col overflow-y-auto">
          <TriageTodoList
            isLoading={todosQuery.isLoading}
            onComplete={(todo) => handleTodoResolve(todo, "done")}
            onDismiss={(todo) => handleTodoResolve(todo, "dismissed")}
            onOpenThread={(todo) =>
              openThread({
                channelId: todo.channelId,
                eventId: todo.eventId,
                threadRootId: todo.threadRootId,
              })
            }
            todos={todosQuery.data ?? []}
          />
        </div>
      ) : (
        <div className="mt-3 flex min-h-0 flex-1 flex-row overflow-hidden border-t border-border/60">
          <div className="flex min-h-0 w-80 shrink-0 flex-col overflow-hidden border-r border-border/60">
            <TriageListPane
              emptyMessage={
                activeTab === "important"
                  ? "Nothing needs your attention. Run a scan to check for new unread messages."
                  : "Nothing filtered yet. Items the agent thinks are noise land here so you can correct it."
              }
              isLoading={listIsLoading}
              onSelect={setSelectedEventId}
              selectedEventId={selectedEventId}
              suggestions={visible}
            />
          </div>
          <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
            <TriageDetailPane
              isAdopting={adoptMutation.isPending}
              onAdopt={(suggestion) => void handleAdopt(suggestion)}
              onDismiss={handleDismiss}
              onOpenThread={(suggestion) =>
                openThread({
                  channelId: suggestion.channelId,
                  eventId: suggestion.eventId,
                  threadRootId: suggestion.threadRootId,
                })
              }
              onPromote={handlePromote}
              suggestion={selected}
            />
          </div>
        </div>
      )}
    </div>
  );
}
