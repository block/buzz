import * as React from "react";

import {
  HostAgentdError,
  hostAgentdArm,
  hostAgentdCreateAgent,
  hostAgentdDisarm,
  hostAgentdLocationProof,
  hostAgentdStatus,
  type CreateRemoteAgentInput,
} from "./hostAgentdClient";
import { deriveRemoteAgentCards } from "./deriveRemoteAgentCards";
import {
  clearRemoteHostConnection,
  loadRemoteHostConnection,
  saveRemoteHostConnection,
} from "./remoteHostSettings";
import type {
  HostAgentStatus,
  RemoteAgentCardModel,
  RemoteAgentPreset,
  RemoteHostConnection,
} from "./types";

const POLL_MS = 15_000;

export function useRemoteHostAgents() {
  const [connection, setConnection] =
    React.useState<RemoteHostConnection | null>(() =>
      loadRemoteHostConnection(),
    );
  const [status, setStatus] = React.useState<HostAgentStatus | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [isLoading, setIsLoading] = React.useState(false);
  const [isPending, setIsPending] = React.useState(false);
  const [notice, setNotice] = React.useState<string | null>(null);
  const [pendingSeat, setPendingSeat] = React.useState<string | null>(null);
  const [locationProof, setLocationProof] = React.useState<Record<
    string,
    unknown
  > | null>(null);

  const refresh = React.useCallback(async () => {
    const conn = loadRemoteHostConnection();
    setConnection(conn);
    if (!conn?.baseUrl || !conn.token) {
      setStatus(null);
      setLocationProof(null);
      setError(null);
      return;
    }
    setIsLoading(true);
    try {
      const next = await hostAgentdStatus(conn.baseUrl, conn.token);
      setStatus(next);
      setError(null);
      try {
        // Prefer public place_proof.v1 (no surface_root/pid) — privacy by default
        let proof: Record<string, unknown>;
        try {
          proof = await hostAgentdLocationProof(
            conn.baseUrl,
            conn.token,
            "public",
          );
        } catch {
          proof = await hostAgentdLocationProof(conn.baseUrl, conn.token);
        }
        setLocationProof(proof);
        const proofSeats =
          (proof.seats as Array<Record<string, unknown>>) || [];
        const proofBodies =
          (proof.bodies as Array<Record<string, unknown>>) || [];
        if (next.seats && (proofSeats.length > 0 || proofBodies.length > 0)) {
          next.seats = next.seats.map((s) => {
            const body = proofBodies.find(
              (p) => p.seat_id === s.seat_id || p.legal_name === s.seat_id,
            );
            const match = proofSeats.find((p) => p.seat_id === s.seat_id);
            const health = (body?.health || match?.health) as
              | string
              | undefined;
            return {
              ...s,
              birth_cert_id:
                s.birth_cert_id ||
                (body?.birth_cert_id as string) ||
                (match?.birth_cert_id as string) ||
                (match?.pubkey as string) ||
                s.pubkey ||
                s.pubkey_hint,
              body_id:
                s.body_id ||
                (body?.body_id as string) ||
                (match?.body_id as string),
              lease_epoch:
                s.lease_epoch ??
                (body?.lease_epoch as number | undefined) ??
                (match?.lease_epoch as number | undefined),
              surface_kind:
                s.surface_kind ||
                (body?.surface_kind as string) ||
                (match?.surface_kind as string),
              surface_id:
                s.surface_id ||
                (body?.surface_id as string) ||
                (match?.surface_id as string),
              // Do not merge surface_root into UI status from public proof
              project_ids:
                s.project_ids ||
                (match?.project_ids as string[] | undefined) ||
                [],
              unit_alive:
                s.unit_alive ??
                (health === "ok" || health === "online"
                  ? true
                  : health === "down" || health === "stopped"
                    ? false
                    : s.unit_alive),
            };
          });
          setStatus({ ...next });
        }
      } catch {
        setLocationProof(null);
      }
    } catch (err) {
      const message =
        err instanceof HostAgentdError
          ? err.message
          : err instanceof Error
            ? err.message
            : "status failed";
      setError(message);
      setStatus(null);
      setLocationProof(null);
    } finally {
      setIsLoading(false);
    }
  }, []);

  const hasConnection = Boolean(connection?.baseUrl && connection?.token);

  React.useEffect(() => {
    void refresh();
    if (!hasConnection) return;
    const id = window.setInterval(() => {
      void refresh();
    }, POLL_MS);
    return () => window.clearInterval(id);
  }, [hasConnection, refresh]);

  const saveConnection = React.useCallback(
    (conn: RemoteHostConnection) => {
      saveRemoteHostConnection(conn);
      setConnection(loadRemoteHostConnection());
      setNotice("Host connection saved");
      void refresh();
    },
    [refresh],
  );

  const clearConnection = React.useCallback(() => {
    clearRemoteHostConnection();
    setConnection(null);
    setStatus(null);
    setError(null);
    setNotice("Host connection cleared");
  }, []);

  const arm = React.useCallback(
    async (seatId: string, preset: RemoteAgentPreset, room?: string) => {
      const conn = loadRemoteHostConnection();
      if (!conn) {
        setError("Configure host connection first");
        return;
      }
      setIsPending(true);
      setPendingSeat(seatId);
      setNotice(null);
      try {
        const result = await hostAgentdArm(
          conn.baseUrl,
          conn.token,
          seatId,
          preset,
          room,
        );
        setNotice(
          result.stdout?.split("\n").find((l) => l.includes("BUZZ_HOST")) ||
            `Armed ${seatId} · ${preset}`,
        );
        setError(null);
        await refresh();
      } catch (err) {
        setError(err instanceof Error ? err.message : "arm failed");
      } finally {
        setIsPending(false);
        setPendingSeat(null);
      }
    },
    [refresh],
  );

  const disarm = React.useCallback(
    async (seatId: string, preset: RemoteAgentPreset) => {
      const conn = loadRemoteHostConnection();
      if (!conn) {
        setError("Configure host connection first");
        return;
      }
      setIsPending(true);
      setPendingSeat(seatId);
      setNotice(null);
      try {
        await hostAgentdDisarm(conn.baseUrl, conn.token, seatId, preset);
        setNotice(`Disarmed ${seatId} · ${preset}`);
        setError(null);
        await refresh();
      } catch (err) {
        setError(err instanceof Error ? err.message : "disarm failed");
      } finally {
        setIsPending(false);
        setPendingSeat(null);
      }
    },
    [refresh],
  );

  const createAgent = React.useCallback(
    async (input: CreateRemoteAgentInput) => {
      const conn = loadRemoteHostConnection();
      if (!conn) {
        setError("Configure host connection first");
        throw new Error("Configure host connection first");
      }
      setIsPending(true);
      setPendingSeat(input.seatId || input.displayName || null);
      setNotice(null);
      try {
        const result = await hostAgentdCreateAgent(
          conn.baseUrl,
          conn.token,
          input,
        );
        setNotice(
          result.armed
            ? `Created + armed ${result.seat_id} · ${result.model || input.model}`
            : `Registered ${result.seat_id} on host`,
        );
        setError(null);
        await refresh();
        return result;
      } catch (err) {
        const message =
          err instanceof Error ? err.message : "create remote agent failed";
        setError(message);
        throw err instanceof Error ? err : new Error(message);
      } finally {
        setIsPending(false);
        setPendingSeat(null);
      }
    },
    [refresh],
  );

  const cards: RemoteAgentCardModel[] = React.useMemo(
    () =>
      deriveRemoteAgentCards(status, Boolean(error && !status), locationProof),
    [status, error, locationProof],
  );

  return {
    connection,
    status,
    locationProof,
    error,
    notice,
    isLoading,
    isPending,
    pendingSeat,
    cards,
    refresh,
    saveConnection,
    clearConnection,
    arm,
    disarm,
    createAgent,
    setNotice,
    setError,
  };
}
