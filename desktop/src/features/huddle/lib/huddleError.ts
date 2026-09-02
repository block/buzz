export type HuddleAction = "join" | "start";

const HUDDLE_ERROR_MESSAGES: ReadonlyArray<{
  codes: readonly string[];
  message: string;
}> = [
  {
    codes: [
      "huddle_audio_unavailable",
      "huddle audio unavailable in this deployment",
    ],
    message:
      "Huddle audio isn’t available on this server. Ask an administrator to turn it on.",
  },
  { codes: ["room_full"], message: "This huddle is full." },
  {
    codes: ["room_ended", "channel is archived"],
    message: "This huddle has ended.",
  },
  {
    codes: ["huddle_relay_draining"],
    message: "The huddle relay is restarting. Reconnecting…",
  },
  {
    codes: ["huddle_owner_unreachable"],
    message: "The huddle relay can’t be reached. Try again in a moment.",
  },
  {
    codes: ["unsupported_version"],
    message: "Update Buzz to join this huddle.",
  },
  {
    codes: ["upgrade_required"],
    message:
      "This huddle uses a newer audio version. Update Buzz, then try again.",
  },
];

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

  const mapped = HUDDLE_ERROR_MESSAGES.find(({ codes }) =>
    codes.some((code) => normalized?.includes(code)),
  );
  if (mapped) return mapped.message;

  if (message) {
    return message;
  }

  return action === "join"
    ? "Couldn’t join the huddle."
    : "Couldn’t start the huddle.";
}
