export type HuddleAction = "join" | "start";

const HUDDLE_AUDIO_UNAVAILABLE_MESSAGE =
  "Huddle audio isn’t available on this server. Ask an administrator to turn it on.";

function rawErrorMessage(error: unknown): string | null {
  if (error instanceof Error) {
    return error.message;
  }
  if (typeof error === "string") {
    return error;
  }
  return null;
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

  if (message) {
    return message;
  }

  return action === "join"
    ? "Couldn’t join the huddle."
    : "Couldn’t start the huddle.";
}
