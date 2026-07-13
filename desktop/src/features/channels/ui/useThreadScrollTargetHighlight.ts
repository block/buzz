import * as React from "react";

export function useThreadScrollTargetHighlight(
  threadScrollTargetId: string | null,
  setThreadScrollTargetId: React.Dispatch<React.SetStateAction<string | null>>,
) {
  const [threadHighlightTargetId, setThreadHighlightTargetId] = React.useState<
    string | null
  >(null);
  const resolveThreadScrollTarget = React.useCallback(() => {
    setThreadScrollTargetId(null);
    setThreadHighlightTargetId(null);
  }, [setThreadScrollTargetId]);

  return {
    resolveThreadScrollTarget,
    setThreadHighlightTargetId,
    threadScrollTargetHighlighted:
      threadScrollTargetId !== null &&
      threadScrollTargetId === threadHighlightTargetId,
  };
}
