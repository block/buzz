import * as React from "react";
import { toast } from "sonner";

import { FibreDetailPane } from "@/features/home/ui/fibre/FibreDetailPane";
import { FibreListPane } from "@/features/home/ui/fibre/FibreListPane";
import {
  collectFibrePubkeys,
  primaryThreadTarget,
} from "@/features/home/ui/fibre/fibreFormat";
import { HomeLoadingState } from "@/features/home/ui/HomeLoadingState";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import type { Fibre } from "@/features/triage/api";
import {
  useFibreFeedbackMutation,
  useFibresQuery,
  usePatchFibreMutation,
  useRestoreFibresMutation,
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
  const fibresQuery = useFibresQuery(currentPubkey);
  const patchMutation = usePatchFibreMutation(currentPubkey);
  const restoreMutation = useRestoreFibresMutation(currentPubkey);
  const feedbackMutation = useFibreFeedbackMutation();
  const [selectedId, setSelectedId] = React.useState<string | null>(null);

  const fibres = fibresQuery.data?.fibres ?? [];
  const clearedCount = fibresQuery.data?.clearedCount ?? 0;
  const profilePubkeys = React.useMemo(() => {
    const pubkeys = collectFibrePubkeys(fibres);
    if (currentPubkey) pubkeys.push(currentPubkey);
    return pubkeys;
  }, [currentPubkey, fibres]);
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
  }, [fibres, mark, onOpenContext, selected]);

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
      data-testid="fibre-inbox"
    >
      <FibreListPane
        clearedCount={clearedCount}
        currentPubkey={currentPubkey}
        fibres={fibres}
        nowMs={nowMs}
        onSelect={setSelectedId}
        profiles={profiles}
        selectedId={selected?.id ?? null}
      />
      <FibreDetailPane
        currentPubkey={currentPubkey}
        fibre={selected}
        isZero={fibres.length === 0}
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
        onRestore={() => {
          restoreMutation.mutate(undefined, {
            onSuccess: () => toast.success("Restored triaged fibres"),
            onError: (error) =>
              toast.error(
                error instanceof Error ? error.message : "Restore failed",
              ),
          });
        }}
      />
    </div>
  );
}
