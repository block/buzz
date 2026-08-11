type PendingAuthChallenge = {
  challenge: string;
  generation: number;
};

type RelayAuthEvent = { id: string };

/** Holds a NIP-42 challenge that arrived before the native socket id returned. */
export class RelayAuthChallengeBuffer {
  #pending: PendingAuthChallenge | null = null;

  defer(challenge: string, generation: number): void {
    this.#pending = { challenge, generation };
  }

  async prepare<T extends RelayAuthEvent>(
    challenge: string,
    generation: number,
    currentGeneration: () => number,
    isReady: () => boolean,
    createEvent: () => Promise<T>,
  ): Promise<T | null> {
    if (generation !== currentGeneration()) return null;
    if (!isReady()) {
      this.defer(challenge, generation);
      return null;
    }

    const event = await createEvent();
    return generation === currentGeneration() && isReady() ? event : null;
  }

  take(generation: number): string | null {
    const pending = this.#pending;
    this.#pending = null;
    return pending?.generation === generation ? pending.challenge : null;
  }

  clear(): void {
    this.#pending = null;
  }
}
