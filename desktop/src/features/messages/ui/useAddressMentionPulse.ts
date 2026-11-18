import * as React from "react";

export function useAddressMentionPulse() {
  const [pulseVersionByPubkey, setPulseVersionByPubkey] = React.useState<
    Record<string, number>
  >({});
  const [shakeVersionByPubkey, setShakeVersionByPubkey] = React.useState<
    Record<string, number>
  >({});
  const pulseMany = React.useCallback((pubkeys: readonly string[]) => {
    setPulseVersionByPubkey((current) => {
      const next = { ...current };
      for (const pubkey of new Set(
        pubkeys.map((value) => value.toLowerCase()),
      )) {
        next[pubkey] = (next[pubkey] ?? 0) + 1;
      }
      return next;
    });
  }, []);
  const pulseOne = React.useCallback(
    (pubkey: string) => pulseMany([pubkey]),
    [pulseMany],
  );
  const shakeMany = React.useCallback((pubkeys: readonly string[]) => {
    setShakeVersionByPubkey((current) => {
      const next = { ...current };
      for (const pubkey of new Set(
        pubkeys.map((value) => value.toLowerCase()),
      )) {
        next[pubkey] = (next[pubkey] ?? 0) + 1;
      }
      return next;
    });
  }, []);

  return {
    pulseMany,
    pulseOne,
    pulseVersionByPubkey,
    shakeMany,
    shakeVersionByPubkey,
  };
}
