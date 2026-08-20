import type { RelayEvent } from "@/shared/api/types";
import { invokeTauri, type StampedArtifact } from "./tauri";

export async function decryptObserverEvent(
  event: RelayEvent,
): Promise<unknown> {
  // C6/C7: thread {value, artifact} to application sites — unwrap is temporary.
  const { value } = await invokeTauri<StampedArtifact<unknown>>(
    "decrypt_observer_event",
    { eventJson: JSON.stringify(event) },
  );
  return value;
}

export async function buildObserverControlEvent(input: {
  agentPubkey: string;
  payload: unknown;
}): Promise<RelayEvent> {
  // C6/C7: thread {value, artifact} to application sites — unwrap is temporary.
  const { value } = await invokeTauri<StampedArtifact<string>>(
    "build_observer_control_event",
    { agentPubkey: input.agentPubkey, payload: input.payload },
  );
  return JSON.parse(value) as RelayEvent;
}
