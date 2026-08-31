import type { HostRow } from "./registration";
import type { PresenceRun } from "@/features/presence/runPresence";
import { startUnavailable } from "./startSelection";

export type MoveProgress = {
  operation: string;
  agent: string;
  source_host: string;
  source_run: string;
  destination_host: string;
  destination_run: string;
  status: string;
  error?: string;
};

/** Presence chooses an exact generation, never proves its termination. */
export function moveUnavailable(
  source: PresenceRun | undefined,
  destination: HostRow,
  agent: string,
  online: boolean | undefined,
  destinationActive = false,
) {
  if (!source?.location || !/^[a-f0-9]{32}$/.test(source.run))
    return "Select an active instance with a known host and run";
  if (source.location.host === destination.host) return "Already on this host";
  if (destinationActive)
    return "Agent already has an active instance on this host";
  return startUnavailable(destination, agent, online);
}

export function moveStatus(status: string): string {
  switch (status) {
    case "stopping":
      return "Stopping only the selected instance; destination has not started.";
    case "stop_unconfirmed":
      return "Source termination unconfirmed. Move is blocked; no destination Start. Retry keeps the exact saved Stop.";
    case "stopped_waiting_destination":
      return "Source confirmed stopped. Waiting for destination setup or authorization; refresh and retry this Move. Source will not restart automatically.";
    case "starting":
      return "Source confirmed stopped. Destination Start queued; waiting for its outcome.";
    case "stopped_start_rejected":
      return "Source confirmed stopped; destination rejected Start. Fix destination setup, then create a new Start attempt on that host.";
    case "destination_spawned":
      return "Source confirmed stopped; destination process spawned in a fresh session. Readiness is not yet confirmed.";
    case "destination_listening":
      return "Source confirmed stopped; destination harness listening. Readiness is not yet confirmed.";
    case "destination_ready":
      return "Source confirmed stopped; destination reported ready in a fresh session.";
    default:
      return "Source confirmed stopped; destination outcome unknown. Retry the saved Start, never a replacement or automatic source restart.";
  }
}
