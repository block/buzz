/**
 * The relay answered a publish with an explicit refusal: a NIP-01 `OK` frame
 * whose success flag is `false`, carrying the relay's own reason.
 *
 * Every other publish failure (a timeout, a dropped socket, a send error, a
 * canceled send) says nothing about what the relay did, so it is *ambiguous*:
 * the relay may have stored the event and lost only the acknowledgement.
 */
export class RelayEventRejectedError extends Error {
  readonly reason: string;

  constructor(reason: string) {
    super(reason);
    this.name = "RelayEventRejectedError";
    this.reason = reason;
  }
}

/**
 * NIP-01 machine-readable prefixes the relay only produces for an event it
 * refused before storing it. `error:` (an internal failure) is absent on
 * purpose: a backend fault can strike after the commit.
 */
const REFUSAL_PREFIXES = [
  "invalid:",
  "conflict:",
  "restricted:",
  "blocked:",
  "auth-required:",
  "rate-limited:",
];

/**
 * True only when `error` is the relay explicitly refusing the event with a
 * reason that means "not stored". Such a refusal is the one proof, short of
 * reading the relay back, that a publish did not commit.
 */
export function isDefinitiveRelayRefusal(error: unknown): boolean {
  return (
    error instanceof RelayEventRejectedError &&
    REFUSAL_PREFIXES.some((prefix) => error.reason.startsWith(prefix))
  );
}
