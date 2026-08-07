import * as React from "react";
import { toast } from "sonner";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useManagedAgentLogQuery } from "@/features/agents/hooks";
import {
  useManagedAgentRuntimeAction,
  useManagedAgentRuntimesQuery,
} from "@/features/agents/managedAgentRuntimeHooks";
import { useChannelsQuery } from "@/features/channels/hooks";
import { useCommunities } from "@/features/communities/useCommunities";
import { useSendMessageMutation } from "@/features/messages/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import type {
  ManagedAgent,
  ManagedAgentRuntimeStatus,
} from "@/shared/api/types";

import {
  filterEligibleRapidTestChannels,
  RapidPostSaveRouteError,
  runRapidAgentPostSaveAction,
  type RapidSaveMode,
  type RapidTestSelection,
} from "./agentRapidTest";
import { findManagedAgentRuntime } from "../managedAgentRuntimeStatus";
import {
  abortRapidActionLeaseForMount,
  claimRapidActionLease,
  createRapidActionMountId,
  finishRapidActionLease,
  releaseRapidActionMount,
  startRapidActionLease,
  type RapidActionLeaseScope,
} from "../rapidActionLease";

const RUNTIME_READY_TIMEOUT_MS = 20_000;
const RUNTIME_READY_POLL_MS = 250;

function assertRapidActionNotAborted(signal: AbortSignal) {
  if (signal.aborted) {
    throw new Error("Rapid agent action cancelled.");
  }
}

function assertRapidActionRoute(route: string): void {
  if (currentRapidActionRoute() !== route) {
    throw new Error("Rapid agent action route changed.");
  }
}

function currentRapidActionRoute(): string {
  return typeof globalThis.location === "undefined"
    ? ""
    : globalThis.location.hash;
}

function waitForTimer(milliseconds: number): Promise<void> {
  return new Promise((resolve) => {
    globalThis.setTimeout(resolve, milliseconds);
  });
}

export function useAgentRapidIteration({
  agentBackend,
  agentPubkey,
  onClose,
  open,
}: {
  agentBackend: ManagedAgent["backend"];
  agentPubkey: string;
  onClose: () => void;
  open: boolean;
}) {
  const runtimeActionMutation = useManagedAgentRuntimeAction();
  const { activeCommunity } = useCommunities();
  const { goChannel } = useAppNavigation();
  const identityQuery = useIdentityQuery();
  const ownerIdentity = identityQuery.data;
  const ownerIdentityPubkey = ownerIdentity?.pubkey ?? null;
  const refetchIdentity = identityQuery.refetch;
  const identityRefetchKeyRef = React.useRef<string | null>(null);
  const actionAbortRef = React.useRef<AbortController | null>(null);
  const rapidMountIdRef = React.useRef<string | null>(null);
  const rapidMountId =
    rapidMountIdRef.current ?? createRapidActionMountId();
  rapidMountIdRef.current = rapidMountId;
  const channelsQuery = useChannelsQuery({ enabled: open });
  const sendMessageMutation = useSendMessageMutation(null, ownerIdentity);
  const [selection, setSelection] = React.useState<RapidTestSelection | null>(
    null,
  );
  const [action, setAction] = React.useState<RapidSaveMode | null>(null);
  const [error, setError] = React.useState<string | null>(null);

  const isLocalAgent = agentBackend.type === "local";
  const activeRelayUrl = activeCommunity?.relayUrl?.trim() || null;
  const rapidScopeRef = React.useRef<{
    activeRelayUrl: string | null;
    agentPubkey: string;
    open: boolean;
    ownerIdentityPubkey: string | null;
  } | null>(null);
  const runtimesQuery = useManagedAgentRuntimesQuery({
    enabled: open && isLocalAgent,
    refetchInterval: open && isLocalAgent && action === null ? 3_000 : false,
  });
  const runtime = React.useMemo<ManagedAgentRuntimeStatus | null>(() => {
    if (!activeRelayUrl) {
      return null;
    }
    return (
      findManagedAgentRuntime(
        runtimesQuery.data ?? [],
        agentPubkey,
        activeRelayUrl,
      ) ?? null
    );
  }, [activeRelayUrl, agentPubkey, runtimesQuery.data]);
  const logQuery = useManagedAgentLogQuery(
    open && isLocalAgent ? agentPubkey : null,
  );

  const hasActiveRelay = Boolean(activeRelayUrl);
  const canRestart = isLocalAgent && hasActiveRelay;
  const canSmoke =
    canRestart &&
    ownerIdentity != null &&
    !ownerIdentity.locked &&
    !ownerIdentity.lost &&
    !ownerIdentity.resetFailed;
  const actionRelayUrl = runtime?.relayUrl ?? activeRelayUrl;

  React.useEffect(() => {
    if (!open) {
      identityRefetchKeyRef.current = null;
      return;
    }
    if (
      identityQuery.data != null ||
      identityRefetchKeyRef.current === agentPubkey
    ) {
      return;
    }

    // The identity query can fail during startup and cache the empty result.
    // Retry a small, bounded number of times per opened agent so a now-ready
    // identity enables the smoke action without creating an idle poll loop.
    identityRefetchKeyRef.current = agentPubkey;
    let cancelled = false;
    void (async () => {
      for (let attempt = 0; attempt < 3 && !cancelled; attempt += 1) {
        if (attempt > 0) {
          await new Promise((resolve) => window.setTimeout(resolve, 750));
        }
        const result = await identityQuery.refetch();
        if (cancelled || result.data != null) {
          return;
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [agentPubkey, identityQuery.data, identityQuery.refetch, open]);

  React.useEffect(() => {
    const previous = rapidScopeRef.current;
    const stableOwnerIdentityPubkey =
      ownerIdentityPubkey ?? previous?.ownerIdentityPubkey ?? null;
    const contextChanged =
      previous != null &&
      (previous.activeRelayUrl !== activeRelayUrl ||
        previous.agentPubkey !== agentPubkey ||
        previous.open !== open ||
        (ownerIdentityPubkey != null &&
          previous.ownerIdentityPubkey != null &&
          previous.ownerIdentityPubkey !== ownerIdentityPubkey));

    rapidScopeRef.current = {
      activeRelayUrl,
      agentPubkey,
      open,
      ownerIdentityPubkey: stableOwnerIdentityPubkey,
    };

    if (!open || contextChanged) {
      abortRapidActionLeaseForMount(rapidMountId);
      actionAbortRef.current?.abort();
      actionAbortRef.current = null;
      setSelection(null);
      setAction(null);
      setError(null);
    }
  }, [activeRelayUrl, agentPubkey, open, ownerIdentityPubkey, rapidMountId]);

  React.useEffect(() => {
    if (open) {
      const scope: RapidActionLeaseScope = {
        activeRelayUrl,
        agentPubkey,
        ownerIdentityPubkey,
        route: currentRapidActionRoute(),
      };
      claimRapidActionLease(scope, rapidMountId);
    }

    return () => {
      releaseRapidActionMount(rapidMountId);
    };
  }, [activeRelayUrl, agentPubkey, open, ownerIdentityPubkey, rapidMountId]);

  const waitForReady = React.useCallback(
    async (signal: AbortSignal) => {
      if (!canRestart || !activeRelayUrl) {
        throw new Error("runtime unavailable");
      }

      const deadline = Date.now() + RUNTIME_READY_TIMEOUT_MS;
      while (Date.now() < deadline) {
        assertRapidActionNotAborted(signal);
        const refreshed = await runtimesQuery.refetch();
        assertRapidActionNotAborted(signal);
        const current = findManagedAgentRuntime(
          refreshed.data ?? [],
          agentPubkey,
          activeRelayUrl,
        );

        if (current?.lifecycle === "ready") {
          return;
        }
        if (
          current?.lifecycle === "failed" ||
          current?.lifecycle === "stopped"
        ) {
          throw new Error("runtime failed");
        }
        await waitForTimer(RUNTIME_READY_POLL_MS);
      }

      throw new Error("runtime readiness timeout");
    },
    [activeRelayUrl, agentPubkey, canRestart, runtimesQuery],
  );

  const run = React.useCallback(
    async (
      mode: RapidSaveMode,
      save: (
        markRecordSaved: () => void,
        signal: AbortSignal,
        assertActionAuthority: () => void,
      ) => Promise<ManagedAgent>,
    ): Promise<void> => {
      if (mode !== "save" && !canRestart) {
        setError(
          isLocalAgent
            ? "Choose an active community before restarting or testing this agent."
            : "Restart and smoke testing are available only for local agents.",
        );
        return;
      }
      if (mode === "smoke" && !canSmoke) {
        setError("The owner identity is not available for a smoke test.");
        return;
      }
      let saved = false;
      let ownerMessageAccepted = false;
      const actionRoute = currentRapidActionRoute();
      const actionOwnerPubkey = ownerIdentityPubkey;
      const actionScope: RapidActionLeaseScope = {
        activeRelayUrl,
        agentPubkey,
        ownerIdentityPubkey: actionOwnerPubkey,
        route: actionRoute,
      };
      const actionLease = startRapidActionLease(actionScope, rapidMountId);
      if (!actionLease) {
        setError("A rapid agent action is already in progress for this agent.");
        return;
      }
      const { controller: abortController, id: actionLeaseId } = actionLease;
      actionAbortRef.current = abortController;
      const { signal } = abortController;
      const abortOnRouteChange = () => {
        if (
          mode !== "save" &&
          !ownerMessageAccepted &&
          currentRapidActionRoute() !== actionRoute
        ) {
          abortController.abort();
        }
      };
      window.addEventListener("hashchange", abortOnRouteChange);
      setAction(mode);
      setError(null);
      runtimeActionMutation.reset();
      try {
        const assertActionAuthority = () => {
          assertRapidActionNotAborted(signal);
          if (mode !== "save") {
            assertRapidActionRoute(actionRoute);
          }
        };
        const savedAgent = await save(
          () => {
            saved = true;
          },
          signal,
          assertActionAuthority,
        );
        assertActionAuthority();
        const outcome = await runRapidAgentPostSaveAction({
          mode,
          pubkey: savedAgent.pubkey,
          relayUrl: actionRelayUrl,
          selection,
          restart: async (pubkey, relayUrl) => {
            assertRapidActionNotAborted(signal);
            assertRapidActionRoute(actionRoute);
            await runtimeActionMutation.mutateAsync({
              action: "restart",
              pubkey,
              relayUrl,
            });
            assertRapidActionNotAborted(signal);
            assertRapidActionRoute(actionRoute);
          },
          waitForReady:
            mode === "save"
              ? undefined
              : async () => {
                  await waitForReady(signal);
                  assertRapidActionRoute(actionRoute);
                },
          sendOwnerMessage: async (
            capturedChannel,
            content,
            mentionPubkeys,
          ) => {
            assertRapidActionNotAborted(signal);
            assertRapidActionRoute(actionRoute);
            const refreshedIdentity = await refetchIdentity();
            assertRapidActionNotAborted(signal);
            assertRapidActionRoute(actionRoute);
            if (
              refreshedIdentity.isError ||
              !refreshedIdentity.data ||
              refreshedIdentity.data.locked ||
              refreshedIdentity.data.lost ||
              refreshedIdentity.data.resetFailed ||
              refreshedIdentity.data.pubkey !== actionOwnerPubkey
            ) {
              throw new Error("The owner identity changed before posting.");
            }
            const refreshed = await channelsQuery.refetch();
            assertRapidActionNotAborted(signal);
            assertRapidActionRoute(actionRoute);
            const currentChannel = filterEligibleRapidTestChannels(
              refreshed.data,
              savedAgent,
            ).find((channel) => channel.id === capturedChannel.id);
            if (!currentChannel) {
              throw new Error("The selected channel is no longer eligible.");
            }
            assertRapidActionRoute(actionRoute);
            const sent = await sendMessageMutation.mutateAsync({
              channelId: currentChannel.id,
              content,
              mentionPubkeys,
              targetChannel: currentChannel,
              abortSignal: signal,
            });
            if (
              !actionOwnerPubkey ||
              sent.pubkey.toLowerCase() !== actionOwnerPubkey.toLowerCase()
            ) {
              throw new Error("The owner identity changed while signing.");
            }
            ownerMessageAccepted = true;
            return { eventId: sent.id };
          },
          openThread: async (channelId, eventId) => {
            assertRapidActionNotAborted(signal);
            assertRapidActionRoute(actionRoute);
            await goChannel(channelId, {
              messageId: eventId,
              thread: eventId,
              threadRootId: eventId,
            });
            assertRapidActionNotAborted(signal);
            const openedRoute = decodeURIComponent(currentRapidActionRoute());
            if (
              !openedRoute.includes(channelId) ||
              !openedRoute.includes(eventId)
            ) {
              throw new Error("The accepted smoke thread route did not open.");
            }
            onClose();
          },
        });
        if (outcome?.kind !== "smoke-posted") {
          assertRapidActionNotAborted(signal);
        }

        if (outcome === null) {
          onClose();
        }
        if (outcome?.kind === "restarted") {
          toast.success(`${savedAgent.name} saved and restarted.`);
        } else if (outcome?.kind === "smoke-posted") {
          if (outcome.threadOpened) {
            toast.success(
              `Smoke prompt posted for ${savedAgent.name}; watch the opened thread for its reply.`,
            );
          } else {
            onClose();
            toast.error(
              "Smoke prompt posted, but Buzz could not open its thread. Open the selected channel manually.",
            );
          }
        }
      } catch (cause) {
        if (signal.aborted) {
          return;
        }
        if (cause instanceof RapidPostSaveRouteError) {
          setError(
            "Changes saved, but the runtime route changed before restart. Review the selected harness before retrying.",
          );
          return;
        }
        if (
          cause instanceof Error &&
          cause.message === "Rapid agent action route changed."
        ) {
          setError(
            saved
              ? "Changes saved, but navigation changed before restart or posting. Return to this agent and retry."
              : "Navigation changed before the rapid agent action started.",
          );
          return;
        }
        setError(
          saved
            ? mode === "smoke"
              ? "Changes saved, but the smoke prompt could not be posted or its thread could not be opened."
              : "Changes saved, but the selected runtime could not be restarted."
            : "Could not save agent changes.",
        );
      } finally {
        window.removeEventListener("hashchange", abortOnRouteChange);
        finishRapidActionLease(actionLeaseId);
        if (actionAbortRef.current === abortController) {
          actionAbortRef.current = null;
          if (saved && mode !== "save") {
            void logQuery.refetch();
          }
          setAction(null);
        }
      }
    },
    [
      actionRelayUrl,
      agentPubkey,
      canSmoke,
      canRestart,
      channelsQuery,
      goChannel,
      isLocalAgent,
      logQuery,
      onClose,
      ownerIdentityPubkey,
      refetchIdentity,
      runtimeActionMutation,
      sendMessageMutation,
      selection,
      waitForReady,
    ],
  );

  const cancel = React.useCallback(() => {
    abortRapidActionLeaseForMount(rapidMountId);
    actionAbortRef.current?.abort();
    actionAbortRef.current = null;
    setAction(null);
    setError(null);
  }, [rapidMountId]);

  return {
    action,
    cancel,
    canRestart,
    canSmoke,
    error,
    isPending: runtimeActionMutation.isPending || action !== null,
    logContent: logQuery.data?.content ?? null,
    logError: logQuery.error
      ? new Error("Could not load the managed agent log.")
      : null,
    logLoading: logQuery.isLoading,
    run,
    runtime,
    selection,
    setSelection,
  };
}
