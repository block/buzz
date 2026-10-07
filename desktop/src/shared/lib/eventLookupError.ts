const EVENT_NOT_FOUND_MESSAGE = "event not found";

/** Only the relay's definitive zero-row signal proves that an event was deleted. */
export function isDefinitiveEventNotFound(error: unknown): boolean {
  if (typeof error === "string") {
    return error.includes(EVENT_NOT_FOUND_MESSAGE);
  }
  return (
    error instanceof Error && error.message.includes(EVENT_NOT_FOUND_MESSAGE)
  );
}
