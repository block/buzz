import * as React from "react";

import type { MeshModelCatalog } from "@/shared/api/tauriMesh";
import {
  meshModelCatalog,
  meshStartNode,
  meshStopNode,
} from "@/shared/api/tauriMesh";

import { useManagedAgentsQuery } from "@/features/agents/hooks";
import { deriveMeshCallToAction, usesMeshCompute } from "../meshAgents";
import { activitySample, inferInboundWork } from "../meshActivity";
import { deriveMeshMemoryModel } from "../meshMemoryModel";
import { deriveMeshRowModel } from "../meshRowModel";
import { deriveMeshShareToggle } from "../shareToggleState";
import {
  getShareComputeModel,
  resolveShareComputeModel,
  setShareComputeModel,
  useShareComputeMaxVramGb,
  useShareComputeModel,
} from "../shareComputePreferences";
import { useMeshDownloadProgress } from "./useMeshDownloadProgress";
import { useMeshLiveView } from "./useMeshLiveView";
import { useMeshNodeStatus } from "./useMeshNodeStatus";
import { useMeshServingUsage } from "./useMeshServingUsage";
import { useMeshSnapshot } from "./useMeshSnapshot";

/**
 * Everything the sidebar Compute row and its popover need, resolved once.
 *
 * The row and the popover are two views of one state. Calling the hooks in both
 * would double every poll and let the two disagree mid-flight — the row saying
 * "sharing" while the popover it opened still said "starting". So the state is
 * owned here and passed down.
 */
/**
 * How long a sighting of in-flight work keeps the dot moving.
 *
 * Longer than the 4s usage poll on purpose: a single short request should stay
 * visible past the sample that caught it, or the pulse is a flicker nobody sees.
 */
const ACTIVITY_LATCH_MS = 12_000;

export function useMeshComputeState() {
  const { status, refresh: refreshStatus } = useMeshNodeStatus();
  const selectedModel = useShareComputeModel();
  const maxVramGb = useShareComputeMaxVramGb();
  const [catalog, setCatalog] = React.useState<MeshModelCatalog | null>(null);
  const [pendingAction, setPendingAction] = React.useState<
    "start" | "stop" | null
  >(null);
  const [actionError, setActionError] = React.useState<string | null>(null);
  // Bumping this re-queries the community snapshot right after a local
  // transition, instead of waiting out the 30s poll.
  const [snapshotNonce, setSnapshotNonce] = React.useState(0);

  const {
    snapshot,
    error: snapshotError,
    refresh: refreshSnapshot,
  } = useMeshSnapshot({
    refreshKey: snapshotNonce,
  });

  const toggle = deriveMeshShareToggle(status);
  // Poll whenever a runtime slot is occupied, not just when sharing: a
  // consuming node also has inflight work worth showing.
  const usage = useMeshServingUsage(toggle.isSharing || toggle.isConsuming);
  const { view } = useMeshLiveView(toggle.isSharing || toggle.isConsuming);
  const { progress: downloadProgress, reset: resetDownloadProgress } =
    useMeshDownloadProgress();

  // Reads the resolved `provider` off ManagedAgent, not a persona's raw field:
  // the mesh provider is usually set globally, so the persona value is null for
  // most mesh agents. See meshAgents.ts.
  const { data: managedAgents } = useManagedAgentsQuery();
  const callToAction = deriveMeshCallToAction({
    agents: managedAgents,
    isSharing: toggle.isSharing,
    meshHasCapacity: (snapshot?.sharingDeviceCount ?? 0) > 0,
  });

  // Inbound work has no counter in mesh-llm, so it is inferred by elimination
  // across two samples. Keeping the previous sample in a ref (not state) avoids
  // a re-render purely to remember history.
  const sample = activitySample(usage);
  const previousSampleRef = React.useRef<typeof sample>(null);
  const inboundWork = inferInboundWork({
    isSharing: toggle.isSharing,
    current: sample,
    previous: previousSampleRef.current,
  });
  React.useEffect(() => {
    previousSampleRef.current = sample;
  }, [sample]);

  // One-shot catalog fetch supplies hardware capacity and initializes the
  // shared preference only when Settings has never chosen a model.
  React.useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const value = await meshModelCatalog();
        if (!cancelled) {
          setCatalog(value);
          if (getShareComputeModel().trim() === "" && value.recommended) {
            setShareComputeModel(value.recommended);
          }
        }
      } catch {
        // Non-fatal: the switch stays disabled and Settings still works.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // Re-check the community view whenever the local lifecycle settles.
  // biome-ignore lint/correctness/useExhaustiveDependencies: the local lifecycle fields are intentional triggers, not values read in the body — a settled start/stop should refresh the community view immediately
  React.useEffect(() => {
    setSnapshotNonce((current) => current + 1);
  }, [status?.state, status?.mode]);

  const modelToShare = resolveShareComputeModel(
    selectedModel,
    catalog?.recommended,
  );
  const activeModel = status?.modelName ?? status?.modelId ?? modelToShare;
  const memory = deriveMeshMemoryModel({
    view,
    catalog,
    modelRef: activeModel,
  });
  const busyNow = (usage?.inflight ?? 0) > 0;

  /**
   * Recent work, and the moment sharing went live.
   *
   * `busyNow` alone under-reports badly. `inflight` is sampled every 4s, so a
   * request that starts and finishes between two polls is never seen and the dot
   * never moves -- which is why real activity looks like nothing is happening.
   * Latching the last sighting stops us forgetting observed work immediately; it
   * never invents any.
   */
  const [recentActivity, setRecentActivity] = React.useState(false);
  React.useEffect(() => {
    if (busyNow) setRecentActivity(true);
  }, [busyNow]);
  React.useEffect(() => {
    if (!recentActivity) return;
    const timer = setTimeout(() => setRecentActivity(false), ACTIVITY_LATCH_MS);
    return () => clearTimeout(timer);
  }, [recentActivity]);

  // `undefined` while the query is in flight, so the row can distinguish "no
  // mesh agent" from "not known yet" and never throb a nudge that then vanishes.
  const hasMeshAgent =
    managedAgents === undefined
      ? undefined
      : managedAgents.some(usesMeshCompute);

  /**
   * What every share switch shows.
   *
   * Optimistic on purpose: `meshStartNode` does not resolve until the runtime is
   * actually serving, which on a 17 GB model is minutes. A switch that springs
   * back and sits off for that whole time reads as "the click did not work".
   *
   * Derived once and shared, because the row and the popover used to compute
   * this independently -- the row optimistically, the popover from real status --
   * so the two controls for the same action disagreed for the entire load.
   */
  const shareSwitchChecked =
    pendingAction === "start" || (toggle.isSharing && pendingAction !== "stop");

  const row = deriveMeshRowModel({
    snapshot,
    status,
    toggle,
    view,
    pendingAction,
    busyNow,
    recentActivity,
    hasMeshAgent,
  });

  const setSharing = React.useCallback(
    async (next: boolean) => {
      // Never let this tear down a consume session: `isSharing` is the only
      // thing that authorizes a stop, and status can be stale between polls.
      if (!next && !toggle.isSharing) return;
      setActionError(null);
      setPendingAction(next ? "start" : "stop");
      try {
        if (next) {
          if (!modelToShare) {
            throw new Error("Choose a model in Compute settings first");
          }
          const parsed =
            maxVramGb.trim() === "" ? undefined : Number.parseFloat(maxVramGb);
          await meshStartNode({
            mode: "serve",
            modelId: modelToShare,
            maxVramGb:
              typeof parsed === "number" && !Number.isNaN(parsed)
                ? parsed
                : undefined,
          });
        } else {
          await meshStopNode();
        }
        refreshStatus();
        setSnapshotNonce((current) => current + 1);
      } catch (error) {
        setActionError(error instanceof Error ? error.message : String(error));
      } finally {
        setPendingAction(null);
        // Clear a lingering final download event so the next start does not
        // begin by replaying the last run's progress.
        resetDownloadProgress();
      }
    },
    [
      maxVramGb,
      modelToShare,
      refreshStatus,
      resetDownloadProgress,
      toggle.isSharing,
    ],
  );

  return {
    row,
    toggle,
    status,
    snapshot,
    view,
    usage,
    memory,
    callToAction,
    inboundWork,
    busyNow,
    downloadProgress,
    pendingAction,
    shareSwitchChecked,
    modelToShare,
    setSharing,
    refreshSnapshot,
    // A transport failure is worth showing; an empty mesh is not an error and
    // is already expressed by the row tone.
    error: actionError ?? snapshotError,
  };
}
