export type PendingRelayAuthChallenge = {
  challenge: string;
  generation: number;
};

/** Holds the relay's eager AUTH challenge until the socket session is ready. */
export class RelayAuthChallengeBuffer {
  private pending: PendingRelayAuthChallenge | null = null;

  store(challenge: string, generation: number): void {
    this.pending = { challenge, generation };
  }

  take(generation: number): string | null {
    const pending = this.pending;
    this.pending = null;
    return pending?.generation === generation ? pending.challenge : null;
  }

  clear(): void {
    this.pending = null;
  }
}
