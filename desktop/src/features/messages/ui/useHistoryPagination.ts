import * as React from "react";

type Transaction = { channelId: string | null; revision: number | null };

/** Owns page admission from request through deferred DOM/measurement commit.
 * This coordinator never writes a scroll offset; Virtua owns compensation.
 */
export function useHistoryPagination({
  channelId,
  fetchOlder,
  canLoad,
  renderedRevision,
  renderedChannelId = channelId ?? null,
  scrollElementRef,
  fillViewport = false,
}: {
  channelId?: string | null;
  fetchOlder?: () => Promise<number | undefined>;
  canLoad: boolean;
  renderedRevision: number;
  renderedChannelId?: string | null;
  /** Allow at most three serial pages to fill a viewport without scroll range. */
  fillViewport?: boolean;
  scrollElementRef: { readonly current: HTMLElement | null };
}) {
  const activeChannel = channelId ?? null;
  const transactionRef = React.useRef<Transaction | null>(null);
  const channelRef = React.useRef(activeChannel);
  const fillRef = React.useRef({ count: 0, lastRevision: -1 });
  const [transaction, setTransaction] = React.useState<Transaction | null>(
    null,
  );
  if (channelRef.current !== activeChannel) {
    channelRef.current = activeChannel;
    transactionRef.current = null;
    fillRef.current = { count: 0, lastRevision: -1 };
  }
  const cancel = React.useCallback(() => {
    transactionRef.current = null;
    setTransaction(null);
  }, []);
  React.useEffect(
    () => () => {
      transactionRef.current = null;
    },
    [],
  );

  const start = React.useCallback(() => {
    if (!fetchOlder || !canLoad || transactionRef.current) return false;
    const request: Transaction = { channelId: activeChannel, revision: null };
    // Synchronous reservation closes the gap before React publishes loading.
    transactionRef.current = request;
    setTransaction(request);
    void (async () => {
      try {
        const revision = await fetchOlder();
        if (transactionRef.current !== request) return;
        if (revision === undefined) {
          // No-op, exhausted, canceled or a legacy non-window pager.
          cancel();
          return;
        }
        const received = { ...request, revision };
        transactionRef.current = received;
        setTransaction(received);
      } catch (error) {
        if (transactionRef.current !== request) return;
        console.error("Failed to load timeline history", activeChannel, error);
        cancel();
      }
    })();
    return true;
  }, [activeChannel, canLoad, cancel, fetchOlder]);

  React.useLayoutEffect(() => {
    if (
      !transaction ||
      transactionRef.current !== transaction ||
      transaction.revision === null ||
      renderedChannelId !== transaction.channelId ||
      renderedRevision < transaction.revision
    )
      return;
    const scroller = scrollElementRef.current;
    if (!scroller) {
      cancel();
      return;
    }
    let frame = 0;
    let stableFrames = 0;
    let previous = "";
    const watch = () => {
      if (transactionRef.current !== transaction) return;
      const geometry = `${scroller.scrollTop}:${scroller.scrollHeight}:${scroller.clientHeight}`;
      stableFrames = geometry === previous ? stableFrames + 1 : 0;
      previous = geometry;
      if (stableFrames >= 3) {
        cancel();
        return;
      }
      frame = requestAnimationFrame(watch);
    };
    frame = requestAnimationFrame(watch);
    return () => cancelAnimationFrame(frame);
  }, [
    cancel,
    renderedChannelId,
    renderedRevision,
    scrollElementRef,
    transaction,
  ]);

  const isPending =
    transaction !== null &&
    transactionRef.current === transaction &&
    transaction.channelId === activeChannel;

  React.useEffect(() => {
    // State restarts fill when visual acknowledgement clears the reservation;
    // the ref also closes the synchronous gap before that state render.
    if (!fillViewport || !canLoad || isPending || transactionRef.current)
      return;
    const fill = fillRef.current;
    if (fill.count >= 3 || fill.lastRevision === renderedRevision) return;
    let frame = 0;
    let previous = "";
    let stableFrames = 0;
    const watch = () => {
      const scroller = scrollElementRef.current;
      if (!scroller || scroller.clientHeight <= 0 || transactionRef.current)
        return;
      if (scroller.scrollHeight > scroller.clientHeight + 1) return;
      const geometry = `${scroller.scrollHeight}:${scroller.clientHeight}`;
      stableFrames = geometry === previous ? stableFrames + 1 : 0;
      previous = geometry;
      if (stableFrames >= 3) {
        if (start()) {
          fill.count++;
          fill.lastRevision = renderedRevision;
        }
        return;
      }
      frame = requestAnimationFrame(watch);
    };
    frame = requestAnimationFrame(watch);
    return () => cancelAnimationFrame(frame);
  }, [
    canLoad,
    fillViewport,
    isPending,
    renderedRevision,
    scrollElementRef,
    start,
  ]);

  return { start, cancel, isPending };
}
