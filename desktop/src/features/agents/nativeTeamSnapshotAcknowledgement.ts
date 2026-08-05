type Acknowledge = (id: string) => Promise<boolean>;

/** Serializes native FIFO removal before allowing the bridge to read again. */
export function createNativeTeamSnapshotAcknowledgement(
  acknowledgeNativeSnapshot: Acknowledge,
) {
  let acknowledging = false;

  return {
    async acknowledge(id: string, resumeDrain: () => void): Promise<boolean> {
      acknowledging = true;
      const acknowledged = await acknowledgeNativeSnapshot(id).catch(() => {
        // An IPC failure leaves the native FIFO state unknown. Do not re-read
        // it and risk duplicating a user-visible import or error.
        return null;
      });
      acknowledging = false;
      if (acknowledged === null) return false;

      // `false` means this id is no longer the native FIFO head, so it is safe
      // to check the current head without replaying this request.
      resumeDrain();
      return acknowledged;
    },
    isAcknowledging: () => acknowledging,
    requestDrain: () => !acknowledging,
  };
}
