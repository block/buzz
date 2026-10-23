export type HuddleAction = "join" | "start";

const HUDDLE_AUDIO_UNAVAILABLE_MESSAGE =
  "Huddle audio isn’t available on this server. Ask an administrator to turn it on.";

const HUDDLE_ARCHIVED_MESSAGE =
  "This huddle has ended. Start a new one from the original channel.";

function rawErrorMessage(error: unknown): string | null {
  if (error instanceof Error) {
    return error.message;
  }
  if (typeof error === "string") {
    return error;
  }
  return null;
}

export function isArchivedHuddleChannelError(error: unknown): boolean {
  return (
    rawErrorMessage(error)
      ?.toLowerCase()
      .includes("channel is archived") ?? false
  );
}

export function formatHuddleActionError(
  error: unknown,
  action: HuddleAction,
): string {
  const message = rawErrorMessage(error)?.trim();
  const normalized = message?.toLowerCase();

  if (
    normalized?.includes("huddle_audio_unavailable") ||
    normalized?.includes("huddle audio unavailable in this deployment")
  ) {
    return HUDDLE_AUDIO_UNAVAILABLE_MESSAGE;
  }

  if (normalized?.includes("channel is archived")) {
    return HUDDLE_ARCHIVED_MESSAGE;
  }

  if (message) {
    return message;
  }

  return action === "join"
    ? "Couldn’t join the huddle."
    : "Couldn’t start the huddle.";
}
