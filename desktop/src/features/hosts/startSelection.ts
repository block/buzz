import type { HostRow } from "./registration";

/** Cached availability is only a picker hint. Native rechecks every fact at spawn. */
export function startUnavailable(
  row: HostRow,
  agent: string,
  online: boolean | undefined,
): string | undefined {
  if (online === undefined) return "Destination availability unknown";
  if (!online) return "Destination is offline";
  if (!row.report?.accepts_start)
    return "Destination has no compatible Start receiver";
  if (!agent) return "Choose an agent";
  if (!row.report.provisioned?.some((config) => config.agent === agent))
    return "Set up this same agent identity and compatible configuration on the destination first";
  return undefined;
}

export function startStatus(status: string): string {
  switch (status) {
    case "queued":
      return "Saved locally; waiting for relay";
    case "relay_accepted":
      return "Relay accepted; waiting for destination outcome";
    case "spawned":
      return "Destination process spawned; readiness not yet confirmed";
    case "listening":
      return "Destination harness listening; workload readiness not yet confirmed";
    case "ready":
      return "Destination reported ready";
    case "rejected":
      return "Destination rejected Start. Check its agent setup, compatibility, or an existing run, then refresh and retry.";
    default:
      return "Outcome unknown; retrying the same operation, not launching a replacement";
  }
}
