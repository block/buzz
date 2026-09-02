import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent } from "@/shared/api/tauri";
import { KIND_PROJECT_RELATED_CHANNEL } from "@/shared/constants/kinds";
import { isValidProjectChannelId } from "./projectModels";

function validateProjectCoordinate(projectAddress: string): void {
  if (!/^30621:[0-9a-f]{64}:.+$/.test(projectAddress)) {
    throw new Error("Invalid Project coordinate.");
  }
}

function userFacingWriteError(error: unknown, linked: boolean): unknown {
  if (!(error instanceof Error)) return error;
  if (error.message.startsWith("restricted:")) {
    return new Error(
      `You don't have permission to ${linked ? "link" : "unlink"} channels for this Project.`,
    );
  }
  if (
    error.message.startsWith("invalid:") &&
    /(?:channel|target)/i.test(error.message)
  ) {
    return new Error(
      `This channel can't be ${linked ? "linked to" : "unlinked from"} the Project.`,
    );
  }
  return error;
}

type SetStateDependencies = {
  publishEvent?: typeof relayClient.publishEvent;
  signEvent?: typeof signRelayEvent;
};

/** Submit one desired-state command. The relay serializes the projection. */
export async function setProjectRelatedChannel(
  input: { channelId: string; linked: boolean; projectAddress: string },
  deps: SetStateDependencies = {},
): Promise<void> {
  const channelId = input.channelId.toLowerCase();
  if (!isValidProjectChannelId(channelId)) {
    throw new Error("Channel id must be a canonical UUID.");
  }
  validateProjectCoordinate(input.projectAddress);
  const signEvent = deps.signEvent ?? signRelayEvent;
  const publishEvent =
    deps.publishEvent ?? relayClient.publishEvent.bind(relayClient);

  try {
    const event = await signEvent({
      kind: KIND_PROJECT_RELATED_CHANNEL,
      content: "",
      tags: [
        ["a", input.projectAddress],
        ["op", input.linked ? "add" : "remove"],
        ["d", channelId],
      ],
    });
    await publishEvent(
      event,
      input.linked
        ? "Timed out while linking the channel."
        : "Timed out while unlinking the channel.",
      input.linked
        ? "Failed to link the channel."
        : "Failed to unlink the channel.",
    );
  } catch (error) {
    throw userFacingWriteError(error, input.linked);
  }
}
