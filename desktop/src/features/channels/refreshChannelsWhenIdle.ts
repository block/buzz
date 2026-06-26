type RefreshDeps = {
  /** >0 when the channels query is currently fetching. */
  isFetching: () => number;
  /** Mark the channels query dirty so it refetches. */
  invalidate: () => void;
  /** Re-arm the trailing debounce to retry after the next quiet window. */
  reArm: () => void;
};

/**
 * Refresh the channel list without overlapping or dropping a dirty signal.
 *
 * If get_channels is mid-flight, invalidating now would be silently undone: the
 * in-flight fetch resolves with pre-event data and clears the dirty flag. So we
 * re-arm instead and let a clean refetch land once the query is idle.
 */
export function refreshChannelsWhenIdle(deps: RefreshDeps): void {
  if (deps.isFetching() > 0) {
    deps.reArm();
    return;
  }
  deps.invalidate();
}
