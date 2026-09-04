import {
  DesktopLifecycleControl,
  DesktopLifecycleReceiver,
} from "./DesktopLifecycleControl";
import { DesktopStopControl } from "./DesktopStopControl";
import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useIdentityQuery } from "@/shared/api/hooks";
import { useCommunities } from "@/features/communities/useCommunities";
import { relayClient } from "@/shared/api/relayClient";
import { Button } from "@/shared/ui/button";
import { refreshDesktopList, type DesktopList } from "../desktopList";
import {
  DESKTOP_PULSE_MS,
  desktopFreshness,
  useDesktopObservations,
  type DesktopObservation,
} from "../desktopObservations";

import {
  useDesktopCapabilities,
  type DesktopCapabilities,
} from "../desktopCapabilities";
import { DesktopCapabilityDetails } from "./DesktopCapabilityDetails";

type View = {
  scope?: import("../desktopList").DesktopScope;
  capabilities?: DesktopCapabilities[];
  capabilityWarning?: string;
  list: DesktopList | null;
  error: boolean;
  loading: boolean;
  refresh: () => void;
  observations?: DesktopObservation[];
  observationWarning?: string;
  now?: number;
};

function useDesktopScope() {
  const owner = useIdentityQuery().data?.pubkey;
  const { activeCommunity } = useCommunities();
  const community = activeCommunity?.relayUrl
    .trim()
    .replace(/^http/, "ws")
    .replace(/\/+$/, "");
  return owner && community ? { owner, community } : null;
}

function useDesktopList() {
  const scope = useDesktopScope();
  const { owner, community } = scope ?? {};
  return useQuery({
    queryKey: ["desktop-profiles", owner, community],
    enabled: !!owner && !!community,
    queryFn: ({ signal }) => {
      if (!owner || !community) throw new Error("Desktop scope unavailable");
      return refreshDesktopList({ owner, community }, () => !signal.aborted);
    },
    // Last observer removal cancels and evicts decrypted data, including on an
    // account switch within the same community. No previous-key placeholder.
    gcTime: 0,
    retry: false,
    refetchOnWindowFocus: false,
  });
}

/** Startup and Agents share the existing owner/community query cache. */
export function DesktopListStartup() {
  const [epoch, setEpoch] = useState(0);
  const { refetch } = useDesktopList();
  const { refetch: pulse } = useDesktopObservations(useDesktopScope());
  const { refetch: report } = useDesktopCapabilities(useDesktopScope());
  useEffect(() => {
    const timer = setInterval(() => {
      void pulse();
      void report();
    }, DESKTOP_PULSE_MS);
    const unsubscribe = relayClient.subscribeToReconnects(() => {
      setEpoch((n) => n + 1);
      void refetch();
      void pulse();
      void report();
    });
    return () => {
      clearInterval(timer);
      unsubscribe();
    };
  }, [refetch, pulse, report]);
  const scope = useDesktopScope();
  return <DesktopLifecycleReceiver key={`lifecycle:${epoch}`} scope={scope} />;
}

export function KnownDesktops() {
  const query = useDesktopList();
  const observations = useDesktopObservations(useDesktopScope());
  const capabilities = useDesktopCapabilities(useDesktopScope());
  const [now, setNow] = useState(() => Date.now() / 1000);
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now() / 1000), 30_000);
    return () => clearInterval(timer);
  }, []);
  return (
    <DesktopListView
      scope={useDesktopScope() ?? undefined}
      capabilities={capabilities.data?.rows}
      capabilityWarning={
        capabilities.isError
          ? "Capability refresh unavailable. Previous reports are retained."
          : capabilities.data?.partial
            ? "Partial capability reports; missing facts are unknown."
            : capabilities.data?.warning
      }
      list={query.data ?? null}
      observations={observations.data?.rows}
      now={Math.max(now, Date.now() / 1000)}
      observationWarning={
        observations.isError
          ? "Last-heard refresh unavailable. Previous observations are retained."
          : observations.data?.partial
            ? "Partial last-heard observations; missing times are unknown."
            : observations.data?.warning
      }
      loading={query.isFetching}
      error={query.isError}
      refresh={() => {
        void query.refetch();
        void observations.refetch();
        void capabilities.refetch();
      }}
    />
  );
}

export function DesktopListView({
  scope,
  list,
  loading,
  error,
  refresh,
  observations,
  observationWarning,
  capabilities,
  capabilityWarning,
  now = Date.now() / 1000,
}: View) {
  return (
    <section
      aria-label="Known Desktops"
      className="space-y-3 rounded-lg border p-4"
    >
      <div className="flex items-center justify-between">
        <h2 className="text-sm font-semibold">Known Desktops</h2>
        <Button
          size="sm"
          variant="outline"
          disabled={loading}
          onClick={refresh}
        >
          Refresh
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">
        Private to you. Saved profiles do not indicate whether a Desktop is
        online or ready to run agents. Last heard is a Desktop observation, not
        proof that its agents are running or stopped.
      </p>
      {loading && <p role="status">Loading Desktop profiles…</p>}
      {error && (
        <p role="alert">
          Desktop profiles unavailable. Previously loaded profiles are retained.
        </p>
      )}
      {capabilityWarning && <p role="status">{capabilityWarning}</p>}
      {observationWarning && <p role="status">{observationWarning}</p>}
      {list?.warning && <p role="status">{list.warning}</p>}
      {list?.partial && (
        <p role="status">Partial list: showing up to 100 profiles.</p>
      )}
      {list && !list.rows.length && !error && <p>No Desktop profiles found.</p>}
      {scope && list && (
        <DesktopLifecycleControl
          key={`${scope.owner}:${scope.community}`}
          scope={scope}
          desktops={list.rows}
        />
      )}
      <ul className="space-y-2">
        {list?.rows.map((row) => (
          <li key={row.id} className="text-sm">
            <span title={row.id}>{row.name}</span>
            {row.id === list.local && " · This Desktop"}
            <div className="text-xs text-muted-foreground">
              Profile updated{" "}
              <time dateTime={new Date(row.updated * 1000).toISOString()}>
                {new Date(row.updated * 1000).toLocaleString()}
              </time>
            </div>
            <div className="text-xs text-muted-foreground">
              Last heard:{" "}
              {desktopFreshness(
                observations?.find((item) => item.id === row.id)?.heard,
                now,
              )}
            </div>
            {scope && (
              <DesktopStopControl
                key={`${scope.owner}:${scope.community}:${row.id}`}
                scope={scope}
                desktop={row}
              />
            )}
            <DesktopCapabilityDetails
              report={capabilities?.find((item) => item.id === row.id)}
              now={now}
            />
          </li>
        ))}
      </ul>
    </section>
  );
}
