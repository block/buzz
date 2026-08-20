import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent } from "@/shared/api/tauri";
import { KIND_JOB_CANCEL } from "@/shared/constants/kinds";

export async function cancelAgentJob(
  job: {
    jobId: string;
    requestEventId: string;
    targetPubkey: string;
    channelId: string;
  },
  reason = "Cancelled from Buzz Desktop",
): Promise<void> {
  const event = await signRelayEvent({
    kind: KIND_JOB_CANCEL,
    content: JSON.stringify({ schema: 1, job: job.jobId, reason }),
    tags: [
      ["h", job.channelId],
      ["p", job.targetPubkey],
      ["job", job.jobId],
      ["e", job.requestEventId],
    ],
  });
  await relayClient.publishEvent(
    event,
    "Timed out while requesting job cancellation.",
    "Failed to request job cancellation.",
  );
}
