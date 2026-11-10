/**
 * Reads the block size from a ResizeObserver entry, preferring border-box size
 * when available.
 */
export function readElementBlockSize(entry: ResizeObserverEntry): number {
  return entry.borderBoxSize?.[0]?.blockSize ?? entry.contentRect.height;
}

/**
 * Invokes `onSize` immediately and whenever the observed element's block size
 * changes. Returns a cleanup function that disconnects the observer.
 */
export function observeElementBlockSize(
  element: HTMLElement,
  onSize: (size: number) => void,
): () => void {
  onSize(element.getBoundingClientRect().height);

  if (typeof ResizeObserver === "undefined") {
    return () => {};
  }

  const observer = new ResizeObserver(([entry]) => {
    onSize(readElementBlockSize(entry));
  });
  observer.observe(element);

  return () => {
    observer.disconnect();
  };
}

/**
 * Reads the inline size from a ResizeObserver entry, preferring border-box
 * size when available.
 */
export function readElementInlineSize(entry: ResizeObserverEntry): number {
  return entry.borderBoxSize?.[0]?.inlineSize ?? entry.contentRect.width;
}

/**
 * Invokes `onSize` immediately and whenever the observed element's inline size
 * changes. Returns a cleanup function that disconnects the observer.
 */
export function observeElementInlineSize(
  element: HTMLElement,
  onSize: (size: number) => void,
): () => void {
  onSize(element.getBoundingClientRect().width);

  if (typeof ResizeObserver === "undefined") {
    return () => {};
  }

  const observer = new ResizeObserver(([entry]) => {
    onSize(readElementInlineSize(entry));
  });
  observer.observe(element);

  return () => {
    observer.disconnect();
  };
}
