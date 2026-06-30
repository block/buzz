import * as React from "react";

export function useComposerContentState() {
  const contentRef = React.useRef("");
  const [isContentEmpty, setIsContentEmpty] = React.useState(true);

  const setComposerContentFromText = React.useCallback((nextText: string) => {
    setIsContentEmpty((wasEmpty) => {
      const isEmpty = nextText.trim().length === 0;
      return wasEmpty === isEmpty ? wasEmpty : isEmpty;
    });
  }, []);

  const setComposerContent = React.useCallback(
    (nextContent: string) => {
      contentRef.current = nextContent;
      setComposerContentFromText(nextContent);
    },
    [setComposerContentFromText],
  );

  const syncContentRefFromEditorRef = React.useRef<() => string>(
    () => contentRef.current,
  );

  const syncComposerContentFromEditor = React.useCallback(
    () => syncContentRefFromEditorRef.current(),
    [],
  );

  return {
    contentRef,
    isContentEmpty,
    setComposerContent,
    setComposerContentFromText,
    syncComposerContentFromEditor,
    syncContentRefFromEditorRef,
  };
}
