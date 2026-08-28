export function createVisibleChannelOwnership(
  setVisibleChannel: (channelId: string | null) => void,
) {
  const owners = new Map<symbol, string>();

  return {
    acquire(channelId: string) {
      const token = Symbol(channelId);
      owners.set(token, channelId);
      setVisibleChannel(channelId);
      let released = false;

      return () => {
        if (released) return;
        released = true;
        owners.delete(token);
        setVisibleChannel(Array.from(owners.values()).at(-1) ?? null);
      };
    },
  };
}
