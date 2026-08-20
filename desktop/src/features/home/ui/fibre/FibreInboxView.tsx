import * as React from "react";
import { toast } from "sonner";

import { useCommunities } from "@/features/communities/useCommunities";
import { FibreDetailPane } from "@/features/home/ui/fibre/FibreDetailPane";
import { FibreListPane } from "@/features/home/ui/fibre/FibreListPane";
import {
  collectFibrePubkeys,
  primaryThreadTarget,
} from "@/features/home/ui/fibre/fibreFormat";
import {
  sortFibres,
  type FibreListTab,
} from "@/features/home/ui/fibre/fibreSort";
import { useFibreSeenState } from "@/features/home/ui/fibre/useFibreSeenState";
import { useFibreSort } from "@/features/home/ui/fibre/useFibreSort";
import { HomeLoadingState } from "@/features/home/ui/HomeLoadingState";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import type { Fibre } from "@/features/triage/api";
import {
  useFibreFeedbackMutation,
  useFibresQuery,
  usePatchFibreMutation,
} from "@/features/triage/hooks";
import { useNow } from "@/shared/lib/useNow";

type FibreInboxViewProps = {
  currentPubkey?: string;
  onOpenContext: (
    channelId: string,
    messageId: string,
    threadRootId?: string | null,
  ) => void;
};

function isTypingTarget(target: EventTarget | null) {
  if (!(target instanceof HTMLElement)) return false;
  const tag = target.tagName;
  return (
    tag === "INPUT" ||
    tag === "TEXTAREA" ||
    tag === "SELECT" ||
    target.isContentEditable
  );
}

export function FibreInboxView({
  currentPubkey,
  onOpenContext,
}: FibreInboxViewProps) {
  const nowMs = useNow(30_000);
  const { activeCommunity } = useCommunities();
  const fibresQuery = useFibresQuery(currentPubkey);
  const patchMutation = usePatchFibreMutation(currentPubkey);
  const feedbackMutation = useFibreFeedbackMutation();
  const { markSeen, seenAtById } = useFibreSeenState(
    currentPubkey,
    activeCommunity?.relayUrl,
  );
  const [listTab, setListTab] = React.useState<FibreListTab>("open");
  const { setSort, sort } = useFibreSort(listTab);
  const [selectedId, setSelectedId] = React.useState<string | null>(null);

  const openFibres = React.useMemo(
    () => sortFibres(fibresQuery.data?.fibres ?? [], sort, "activity"),
    [fibresQuery.data?.fibres, sort],
  );
  const doneFibres = React.useMemo(
    () => sortFibres(fibresQuery.data?.done ?? [], sort, "updated"),
    [fibresQuery.data?.done, sort],
  );
  const fibres = listTab === "done" ? doneFibres : openFibres;
  const openCount = fibresQuery.data?.openCount ?? openFibres.length;
  const doneCount = fibresQuery.data?.doneCount ?? doneFibres.length;
  const profilePubkeys = React.useMemo(() => {
    const pubkeys = collectFibrePubkeys([...openFibres, ...doneFibres]);
    if (currentPubkey) pubkeys.push(currentPubkey);
    return pubkeys;
  }, [currentPubkey, doneFibres, openFibres]);
  const profilesQuery = useUsersBatchQuery(profilePubkeys, {
    enabled: profilePubkeys.length > 0,
  });
  const profiles = profilesQuery.data?.profiles;
  const selected =
    fibres.find((fibre) => fibre.id === selectedId) ?? fibres[0] ?? null;

  React.useEffect(() => {
    if (selected && selected.id !== selectedId) {
      setSelectedId(selected.id);
    }
    if (!selected) {
      setSelectedId(null);
    }
  }, [selected, selectedId]);

  React.useEffect(() => {
    if (listTab === "open" && selected) {
      markSeen(selected);
    }
  }, [listTab, markSeen, selected]);

  React.useEffect(() => {
    const root = document.documentElement;
    const showWallpaper =
      listTab === "open" && fibresQuery.isSuccess && openCount === 0;
    if (showWallpaper) {
      root.setAttribute("data-inbox-zero", "");
    } else {
      root.removeAttribute("data-inbox-zero");
    }
    return () => {
      root.removeAttribute("data-inbox-zero");
    };
  }, [fibresQuery.isSuccess, listTab, openCount]);

  const advanceAfter = React.useCallback(
    (fibreId: string) => {
      const index = fibres.findIndex((fibre) => fibre.id === fibreId);
      const next = fibres[index + 1] ?? fibres[index - 1] ?? null;
      setSelectedId(next?.id ?? null);
    },
    [fibres],
  );

  const mark = React.useCallback(
    (fibre: Fibre, status: "done" | "dismissed", message: string) => {
      patchMutation.mutate({ id: fibre.id, status });
      if (currentPubkey) {
        feedbackMutation.mutate({
          pubkey: currentPubkey,
          fibreId: fibre.id,
          eventId: fibre.artifacts[0]?.eventId,
          channelId: fibre.channelId,
          authorPubkey: fibre.artifacts[0]?.authorPubkey,
          threadRootId: fibre.artifacts[0]?.threadRootId,
          userAction: status === "done" ? "done" : "dismissed",
          preview: fibre.title,
        });
      }
      toast.success(message);
      advanceAfter(fibre.id);
    },
    [advanceAfter, currentPubkey, feedbackMutation, patchMutation],
  );

  const reopen = React.useCallback(
    (fibre: Fibre) => {
      patchMutation.mutate({ id: fibre.id, status: "open" });
      toast.success("Reopened");
      advanceAfter(fibre.id);
    },
    [advanceAfter, patchMutation],
  );

  React.useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.metaKey || event.ctrlKey || event.altKey) return;
      if (isTypingTarget(event.target)) return;
      const key = event.key.toLowerCase();
      const index = fibres.findIndex((fibre) => fibre.id === selected?.id);
      if (key === "j" && index >= 0 && index < fibres.length - 1) {
        event.preventDefault();
        setSelectedId(fibres[index + 1].id);
        return;
      }
      if (key === "k" && index > 0) {
        event.preventDefault();
        setSelectedId(fibres[index - 1].id);
        return;
      }
      if (!selected) return;
      if (listTab === "done") {
        if (key === "r") {
          event.preventDefault();
          const target = primaryThreadTarget(selected);
          if (target) {
            onOpenContext(
              target.channelId,
              target.messageId,
              target.threadRootId,
            );
          }
        }
        return;
      }
      if (key === "e") {
        event.preventDefault();
        mark(selected, "done", "Marked done");
      } else if (key === "x") {
        event.preventDefault();
        mark(
          selected,
          "dismissed",
          "Marked not a fibre — the model will weight this pattern lower",
        );
      } else if (key === "h") {
        event.preventDefault();
        toast.message("Snooze isn't wired yet");
      } else if (key === "r") {
        event.preventDefault();
        const target = primaryThreadTarget(selected);
        if (target) {
          onOpenContext(
            target.channelId,
            target.messageId,
            target.threadRootId,
          );
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [fibres, listTab, mark, onOpenContext, selected]);

  if (fibresQuery.isLoading && !fibresQuery.data) {
    return <HomeLoadingState />;
  }

  if (fibresQuery.isError && !fibresQuery.data) {
    return (
      <div
        className="flex min-h-0 min-w-0 flex-1 items-center justify-center p-10"
        data-testid="fibre-inbox-error"
      >
        <div className="max-w-sm text-center">
          <div className="text-base font-medium">Could not load fibres</div>
          <p className="mt-2 text-sm leading-relaxed text-muted-foreground">
            {fibresQuery.error instanceof Error
              ? fibresQuery.error.message
              : "Start the fibre engine with scripts/triage-up.sh."}
          </p>
        </div>
      </div>
    );
  }

  return (
    <div
      className="flex min-h-0 min-w-0 flex-1 overflow-hidden"
      data-fibre-inbox-tab={listTab}
      data-testid="fibre-inbox"
    >
      <FibreListPane
        currentPubkey={currentPubkey}
        doneCount={doneCount}
        fibres={fibres}
        listTab={listTab}
        nowMs={nowMs}
        onListTabChange={setListTab}
        onSelect={setSelectedId}
        onSortChange={setSort}
        openCount={openCount}
        profiles={profiles}
        seenAtById={seenAtById}
        selectedId={selected?.id ?? null}
        sort={sort}
      />
      <FibreDetailPane
        currentPubkey={currentPubkey}
        fibre={selected}
        listTab={listTab}
        nowMs={nowMs}
        profiles={profiles}
        onDismiss={(fibre) =>
          mark(
            fibre,
            "dismissed",
            "Marked not a fibre — the model will weight this pattern lower",
          )
        }
        onDone={(fibre) => mark(fibre, "done", "Marked done")}
        onOpenContext={onOpenContext}
        onReopen={reopen}
      />
    </div>
  );
}
