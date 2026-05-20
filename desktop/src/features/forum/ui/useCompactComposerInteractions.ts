import * as React from "react";

type UseCompactComposerInteractionsArgs = {
  compact: boolean;
  onExpand: () => void;
  onPaperclip: () => Promise<unknown>;
};

export function useCompactComposerInteractions({
  compact,
  onExpand,
  onPaperclip,
}: UseCompactComposerInteractionsArgs) {
  const isMediaPickerActiveRef = React.useRef(false);
  const isToolbarInteractionActiveRef = React.useRef(false);

  const handlePaperclipClick = React.useCallback(() => {
    if (compact) onExpand();
    isMediaPickerActiveRef.current = true;
    void onPaperclip().finally(() => {
      isMediaPickerActiveRef.current = false;
    });
  }, [compact, onExpand, onPaperclip]);

  const handleToolbarMouseDown = React.useCallback(() => {
    if (compact) onExpand();
    isToolbarInteractionActiveRef.current = true;
    window.setTimeout(() => {
      isToolbarInteractionActiveRef.current = false;
    }, 0);
  }, [compact, onExpand]);

  const shouldIgnoreBlur = React.useCallback(
    () =>
      isMediaPickerActiveRef.current || isToolbarInteractionActiveRef.current,
    [],
  );

  return { handlePaperclipClick, handleToolbarMouseDown, shouldIgnoreBlur };
}
