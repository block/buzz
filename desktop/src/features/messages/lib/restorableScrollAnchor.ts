export type RestorableScrollAnchor =
  | { kind: "at-bottom" }
  | {
      fallbackScrollTop: number;
      kind: "message";
      messageId: string;
      topOffset: number;
    };

export type RestoredScrollAnchor =
  | { kind: "at-bottom" }
  | { kind: "message"; messageId: string; topOffset: number };

const TRUE_BOTTOM_THRESHOLD_PX = 1;

function isAtTrueBottom(container: HTMLDivElement): boolean {
  return (
    container.scrollHeight - container.clientHeight - container.scrollTop <=
    TRUE_BOTTOM_THRESHOLD_PX
  );
}

function findMessageAnchor(container: HTMLDivElement): RestoredScrollAnchor {
  const containerTop = container.getBoundingClientRect().top;
  const rows = container.querySelectorAll<HTMLElement>("[data-message-id]");

  for (const row of rows) {
    const rect = row.getBoundingClientRect();
    const messageId = row.dataset.messageId;
    if (rect.bottom > containerTop && messageId) {
      return {
        kind: "message",
        messageId,
        topOffset: rect.top - containerTop,
      };
    }
  }

  return { kind: "at-bottom" };
}

/**
 * Captures a stable viewport position for a conversation that is about to
 * unmount. A message id + viewport offset survives content growth above the
 * reader; scrollTop is retained only as a fallback if that row disappears.
 */
export function captureRestorableScrollAnchor(
  container: HTMLDivElement | null,
): RestorableScrollAnchor | null {
  if (!container) return null;
  if (isAtTrueBottom(container)) return { kind: "at-bottom" };

  const anchor = findMessageAnchor(container);
  if (anchor.kind !== "message") {
    return { kind: "at-bottom" };
  }

  return {
    fallbackScrollTop: container.scrollTop,
    kind: "message",
    messageId: anchor.messageId,
    topOffset: anchor.topOffset,
  };
}

/**
 * Restores a captured viewport position before paint. Returns the anchor the
 * scroll owner should retain for subsequent content updates.
 */
export function restoreRestorableScrollAnchor(
  container: HTMLDivElement,
  anchor: RestorableScrollAnchor,
): RestoredScrollAnchor {
  if (anchor.kind === "at-bottom") {
    container.scrollTo({ top: container.scrollHeight, behavior: "auto" });
    return { kind: "at-bottom" };
  }

  const row = container.querySelector<HTMLElement>(
    `[data-message-id="${CSS.escape(anchor.messageId)}"]`,
  );
  if (!row) {
    container.scrollTo({ top: anchor.fallbackScrollTop, behavior: "auto" });
    return findMessageAnchor(container);
  }

  const currentTopOffset =
    row.getBoundingClientRect().top - container.getBoundingClientRect().top;
  container.scrollTop += currentTopOffset - anchor.topOffset;
  return {
    kind: "message",
    messageId: anchor.messageId,
    topOffset: anchor.topOffset,
  };
}
